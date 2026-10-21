//! The inbox, wakeup batching and questions against an in-memory database.

use super::*;
use crate::{ApprovalPreset, NewRun, Runs};

struct Fixture {
    store: Arc<Store>,
    log: EventLog,
    inbox: Inbox,
    questions: Questions,
    runs: Runs,
}

impl Fixture {
    fn new() -> Self {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let log = EventLog::new(store.clone());
        Self {
            inbox: Inbox::new(store.clone()),
            questions: Questions::new(store.clone(), log.clone()),
            runs: Runs::new(store.clone(), log.clone()),
            store,
            log,
        }
    }

    /// A run in a workspace inserted by hand.
    fn run(&self, name: &str) -> i64 {
        let workspace_id = {
            let conn = self.store.lock();
            conn.execute(
                "INSERT INTO projects (root, name, base_branch, created_ms) VALUES (?1, 'repo', 'main', 0)",
                [format!("/repos/{name}")],
            )
            .unwrap();
            let project = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO workspaces (project_id, name, title, branch, path, base, base_ref, base_commit, created_ms)
                 VALUES (?1, ?2, ?2, 'cos/' || ?2, '/wt/' || ?2, 'head', 'main', 'abc', 0)",
                params![project, name],
            )
            .unwrap();
            conn.last_insert_rowid()
        };
        self.runs
            .create(NewRun {
                workspace_id,
                goal: "add rate limiting".into(),
                preset: ApprovalPreset::default(),
                approval_cap: None,
            })
            .unwrap()
            .id
    }
}

fn item(id: i64, bytes: usize) -> InboxItem {
    InboxItem {
        id,
        run_id: 1,
        kind: InboxKind::Message,
        text: "x".repeat(bytes),
        created_ms: 0,
        wakeup_id: None,
        delivered_ms: None,
    }
}

#[test]
fn items_wait_until_delivered_oldest_first() {
    let fixture = Fixture::new();
    let (run, other) = (fixture.run("lisbon"), fixture.run("porto"));
    let first = fixture
        .inbox
        .add(run, InboxKind::Task, "  task 1 merged ")
        .unwrap();
    let second = fixture.inbox.add(run, InboxKind::Message, "hello").unwrap();
    fixture.inbox.add(other, InboxKind::Message, "elsewhere").unwrap();

    assert_eq!(first.text, "task 1 merged");
    assert_eq!(fixture.inbox.runs_waiting().unwrap(), [run, other]);
    let waiting = fixture.inbox.waiting(run).unwrap();
    assert_eq!(
        waiting.iter().map(|item| item.id).collect::<Vec<_>>(),
        [first.id, second.id]
    );

    let wakeup = fixture.inbox.deliver(run, 7, &waiting[..1]).unwrap();
    assert_eq!((wakeup.items, wakeup.bytes, wakeup.agent_id), (1, 13, 7));
    let waiting = fixture.inbox.waiting(run).unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].id, second.id);
    let delivered = &fixture.inbox.items(run).unwrap()[0];
    assert_eq!(delivered.wakeup_id, Some(wakeup.id));
    assert!(delivered.delivered_ms.is_some());
    assert_eq!(fixture.inbox.wakeups_since(run, 0).unwrap(), 1);
    assert_eq!(fixture.inbox.wakeups_since(other, 0).unwrap(), 0);
    assert_eq!(
        fixture.inbox.oldest_wakeup_since(run, 0).unwrap(),
        Some(wakeup.created_ms)
    );
    assert_eq!(fixture.inbox.oldest_wakeup_since(other, 0).unwrap(), None);
}

#[test]
fn long_items_are_cut_so_any_item_fits() {
    let fixture = Fixture::new();
    let run = fixture.run("lisbon");

    let added = fixture
        .inbox
        .add(run, InboxKind::Message, &"é".repeat(MAX_ITEM_BYTES))
        .unwrap();

    assert!(added.text.len() <= MAX_ITEM_BYTES);
    assert!(added.text.ends_with('…'));
}

#[test]
fn a_batch_holds_at_most_20_items_and_24_kib() {
    let many: Vec<InboxItem> = (1..=25).map(|id| item(id, 10)).collect();
    assert_eq!(next_batch(&many).len(), MAX_WAKEUP_ITEMS);

    // 4 KiB each: six fit in 24 KiB, the seventh waits.
    let big: Vec<InboxItem> = (1..=10).map(|id| item(id, 4 * 1024)).collect();
    assert_eq!(next_batch(&big).len(), 6);

    // One item always goes, however big.
    let huge = [item(1, MAX_WAKEUP_BYTES + 1), item(2, 1)];
    assert_eq!(next_batch(&huge).len(), 1);
    assert!(next_batch(&[]).is_empty());
}

#[test]
fn questions_are_asked_answered_once_and_logged() {
    let fixture = Fixture::new();
    let run = fixture.run("lisbon");

    let question = fixture
        .questions
        .ask(
            run,
            Some(9),
            " Redis or memory? ",
            &["redis".into(), " ".into(), "memory".into()],
        )
        .unwrap();
    assert_eq!(question.question, "Redis or memory?");
    assert_eq!(question.choices, ["redis", "memory"]);
    assert_eq!(
        (question.state, question.answer.as_deref()),
        (QuestionState::Open, None)
    );
    assert_eq!(
        fixture.questions.list(Some(QuestionState::Open), None).unwrap(),
        std::slice::from_ref(&question)
    );

    let answered = fixture.questions.answer(question.id, "memory").unwrap();
    assert_eq!(answered.state, QuestionState::Answered);
    assert_eq!(answered.answer.as_deref(), Some("memory"));
    assert!(answered.answered_ms.is_some());
    assert!(matches!(
        fixture.questions.answer(question.id, "redis"),
        Err(RunError::Conflict(_))
    ));
    assert!(
        fixture
            .questions
            .list(Some(QuestionState::Open), None)
            .unwrap()
            .is_empty()
    );
    assert_eq!(fixture.questions.list(None, Some(run)).unwrap().len(), 1);
    assert!(matches!(fixture.questions.get(99), Err(RunError::NotFound(_))));

    let events = fixture.log.after(0, 100).unwrap();
    let asked = events
        .iter()
        .find(|event| event.kind == kind::QUESTION_ASKED)
        .unwrap();
    assert_eq!(asked.run_id.as_deref(), Some(run.to_string().as_str()));
    assert_eq!(asked.data, json!({ "question_id": question.id, "run_id": run }));
    assert!(events.iter().any(|event| event.kind == kind::QUESTION_ANSWERED));
}

#[test]
fn bad_questions_and_answers_are_refused() {
    let fixture = Fixture::new();
    let run = fixture.run("lisbon");

    assert!(matches!(
        fixture.questions.ask(run, None, "  ", &[]),
        Err(RunError::Invalid(_))
    ));
    let choices: Vec<String> = (0..11).map(|n| n.to_string()).collect();
    assert!(matches!(
        fixture.questions.ask(run, None, "pick", &choices),
        Err(RunError::Invalid(_))
    ));
    let question = fixture.questions.ask(run, None, "why?", &[]).unwrap();
    assert!(matches!(
        fixture.questions.answer(question.id, " "),
        Err(RunError::Invalid(_))
    ));
}

#[test]
fn question_json_matches_the_contract() {
    let question = Question {
        id: 4,
        run_id: 2,
        agent_id: Some(9),
        question: "Use Redis or in-memory?".into(),
        choices: vec!["redis".into(), "memory".into()],
        state: QuestionState::Open,
        answer: None,
        created_ms: 0,
        answered_ms: None,
    };

    assert_eq!(
        serde_json::to_value(&question).unwrap(),
        json!({"id": 4, "run_id": 2, "agent_id": 9, "question": "Use Redis or in-memory?",
               "choices": ["redis", "memory"], "state": "open", "answer": null,
               "created_ms": 0, "answered_ms": null})
    );
}

#[test]
fn the_chief_agent_is_recorded_on_the_run() {
    let fixture = Fixture::new();
    let run = fixture.run("lisbon");

    let updated = fixture.runs.set_chief_agent(run, 12).unwrap();

    assert_eq!(updated.chief_agent_id, Some(12));
    assert_eq!(fixture.runs.run(run).unwrap().chief_agent_id, Some(12));
    assert!(matches!(
        fixture.runs.set_chief_agent(999, 1),
        Err(RunError::NotFound(_))
    ));
}

#[test]
fn cut_keeps_whole_characters() {
    assert_eq!(cut("short", 10), "short");
    let cut_text = cut("ééééé", 6);
    assert!(cut_text.len() <= 6, "{cut_text}");
    assert!(cut_text.ends_with('…'));
}
