use std::io::Write;

use super::*;

const WORKER: &str = include_str!("fixtures/worker.jsonl");
const CONTEXT: &str = include_str!("fixtures/context.jsonl");
const CHIEF: &str = include_str!("fixtures/chief.jsonl");
const INTERRUPTED: &str = include_str!("fixtures/interrupted.jsonl");

fn texts(transcript: &Transcript) -> Vec<(Speaker, MessageKind, String)> {
    transcript
        .messages
        .iter()
        .map(|message| {
            let text = match (&message.text, &message.tool) {
                (Some(text), _) => text.clone(),
                (None, Some(tool)) => format!(
                    "{}: {}{}",
                    tool.name,
                    tool.summary,
                    if tool.is_error { " (error)" } else { "" }
                ),
                (None, None) => String::new(),
            };
            (message.role, message.kind, text)
        })
        .collect()
}

#[test]
fn a_worker_turn_reads_as_prompt_tool_and_reply() {
    let transcript = parse(WORKER.as_bytes());
    assert_eq!(
        texts(&transcript),
        vec![
            (
                Speaker::User,
                MessageKind::Text,
                "Task: Add hello note\n\nCreate the file notes/hello.txt containing the single line: hello\n\nCommit your work, then end with DONE or STUCK: <why>.".to_owned()
            ),
            (
                Speaker::Assistant,
                MessageKind::Tool,
                "Bash: mkdir -p notes && printf 'hello\\n' > notes/hello.txt && git add notes/hello.txt && git commit -m \"Add hello note\"".to_owned()
            ),
            (
                Speaker::Assistant,
                MessageKind::Text,
                "I created `notes/hello.txt` containing `hello` and committed it as `8473a58`.\n\nDONE".to_owned()
            ),
        ]
    );
    let numbers: Vec<usize> = transcript.messages.iter().map(|message| message.n).collect();
    assert_eq!(numbers, vec![1, 2, 3]);
    assert_eq!(transcript.next, 3);
    assert_eq!(transcript.messages[0].ts_ms, 1_792_620_650_625);
    assert_eq!(
        transcript.last_reply(),
        Some("I created `notes/hello.txt` containing `hello` and committed it as `8473a58`.\n\nDONE")
    );
}

#[test]
fn a_chief_turn_hides_tool_search_unwraps_wakeups_and_marks_errors() {
    let transcript = parse(CHIEF.as_bytes());
    let rows = texts(&transcript);
    assert_eq!(rows.len(), 6, "{rows:#?}");
    assert_eq!(
        rows[1].2,
        "mcp__cos__board_propose: {\"tasks\":[{\"files\":[\"calc.py\",\"tests/test_calc.py\"],\"instructions\":\"Add subtract(a, b) to calc.py and a test.\",\"title\":\"Add subtract\"}]}"
    );
    assert_eq!(
        rows[3],
        (
            Speaker::User,
            MessageKind::Update,
            "[cos · 2 updates]\n### Agent 15 \"Add subtract\" finished a turn\nDone.".to_owned()
        )
    );
    assert_eq!(
        rows[4].2,
        "mcp__cos__report_write: {\"report\":\"## Run 2\\n\\nDone.\"} (error)"
    );
    assert_eq!(rows[5].2, "Run 2 is done.");
    assert_eq!(transcript.next, 6);
}

#[test]
fn noise_sidechains_and_bad_lines_are_skipped_and_a_partial_line_waits() {
    let transcript = parse(INTERRUPTED.as_bytes());
    assert_eq!(
        texts(&transcript),
        vec![
            (Speaker::User, MessageKind::Text, "List the files".to_owned()),
            (
                Speaker::Assistant,
                MessageKind::Tool,
                "Bash: ls… (error)".to_owned()
            ),
            (
                Speaker::User,
                MessageKind::Text,
                "Try again, with ls -la".to_owned()
            ),
            (
                Speaker::Assistant,
                MessageKind::Text,
                "Running it again.".to_owned()
            ),
            (
                Speaker::Assistant,
                MessageKind::Tool,
                "Read: /repo/README.md".to_owned()
            ),
        ]
    );
    // The Read result is on the unfinished last line, so the call is still
    // open and not part of what's settled.
    assert_eq!(transcript.next, 4);

    let finished = format!("{INTERRUPTED}\"}}]}},\"uuid\":\"u-7\"}}\n");
    let finished = parse(finished.as_bytes());
    assert_eq!(finished.messages.len(), 5);
    assert_eq!(finished.next, 5);
    assert!(
        finished.messages[4]
            .tool
            .as_ref()
            .is_some_and(|tool| !tool.is_error)
    );
}

#[test]
fn after_keeps_later_messages_and_the_same_next() {
    let transcript = parse(CHIEF.as_bytes()).after(4);
    let numbers: Vec<usize> = transcript.messages.iter().map(|message| message.n).collect();
    assert_eq!(numbers, vec![5, 6]);
    assert_eq!(transcript.next, 6);
    assert!(parse(CHIEF.as_bytes()).after(6).messages.is_empty());
}

#[test]
fn an_open_tool_call_before_text_does_not_hold_next_back() {
    let lines = [
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"a","name":"Bash","input":{"command":"true"}}]}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Moving on."}]}}"#,
    ];
    let transcript = parse(format!("{}\n", lines.join("\n")).as_bytes());
    assert_eq!(transcript.messages.len(), 2);
    assert_eq!(transcript.next, 2);
}

#[test]
fn empty_and_missing_files_are_empty_transcripts() {
    assert_eq!(parse(b""), Transcript::default());
    assert_eq!(parse(b"{\"type\":\"user\""), Transcript::default());
    let missing = Cache::default()
        .read(Path::new("/nonexistent/cos/transcript.jsonl"), None)
        .unwrap();
    assert_eq!(missing, Transcript::default());
}

#[test]
fn the_context_is_the_newest_replys_input_in_the_models_window() {
    let transcript = parse(CONTEXT.as_bytes());
    assert_eq!(
        transcript.context,
        Some(Context {
            used_tokens: 3 + 1_200 + 40_000,
            window_tokens: Some(200_000)
        }),
        "the subagent's and the synthetic reply's usage don't count"
    );
    assert_eq!(transcript.messages.len(), 4);

    // An agent started on a `[1m]` model has the 1M window.
    let mut parser = Parser::default();
    parser.feed(CONTEXT.as_bytes());
    let wide = parser.transcript(Some("claude-sonnet-4-5[1m]"));
    assert_eq!(wide.context.unwrap().window_tokens, Some(1_000_000));
    // `after` keeps it: it's for the whole file.
    assert_eq!(wide.clone().after(4).context, wide.context);

    let json = serde_json::to_value(parse(CONTEXT.as_bytes())).unwrap();
    assert_eq!(
        json["context"],
        serde_json::json!({"used_tokens": 41_203, "window_tokens": 200_000})
    );
    assert_eq!(serde_json::to_value(parse(b"")).unwrap()["context"], Value::Null);
}

#[test]
fn context_windows_by_model() {
    for (launch, model, window) in [
        (None, Some("claude-sonnet-4-5-20250929"), Some(200_000)),
        (None, Some("claude-sonnet-4-20250514"), Some(200_000)),
        (None, Some("claude-opus-4-5"), Some(200_000)),
        (None, Some("claude-haiku-4-5-20251001"), Some(200_000)),
        (None, Some("claude-3-5-sonnet-20241022"), Some(200_000)),
        (None, Some("claude-opus-4-6"), Some(1_000_000)),
        (None, Some("claude-sonnet-4-6"), Some(1_000_000)),
        (None, Some("claude-opus-5-5"), Some(1_000_000)),
        (None, Some("claude-haiku-5-5"), Some(1_000_000)),
        (None, Some("claude-fable-5-1"), Some(1_000_000)),
        (Some("opus[1m]"), Some("claude-opus-4-5"), Some(1_000_000)),
        (Some("sonnet"), Some("claude-sonnet-4-5"), Some(200_000)),
        (None, Some("claude-sonnet-4-5[1M]"), Some(1_000_000)),
        (None, Some("gpt-5"), None),
        (None, Some("claude-newname-9"), None),
        (None, None, None),
    ] {
        assert_eq!(context_window(launch, model), window, "{launch:?} {model:?}");
    }
    // A guess the conversation outgrew is dropped.
    let line = r#"{"type":"assistant","message":{"model":"claude-sonnet-4-5","content":[],"usage":{"input_tokens":250000}}}"#;
    let context = parse(format!("{line}\n").as_bytes()).context.unwrap();
    assert_eq!((context.used_tokens, context.window_tokens), (250_000, None));
}

#[test]
fn a_message_read_mid_turn_shows_as_yours() {
    let lines = [
        user_line("Tag the topics"),
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#.to_owned() + "\n",
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"a"}]}}"#.to_owned() + "\n",
        r#"{"type":"attachment","attachment":{"type":"queued_command","prompt":"stop","commandMode":"prompt"},"timestamp":"2026-10-25T14:38:00Z"}"#.to_owned() + "\n",
        r#"{"type":"attachment","attachment":{"type":"queued_command","prompt":[{"type":"text","text":"and say why"}]}}"#.to_owned() + "\n",
        r#"{"type":"attachment","attachment":{"type":"todo_reminder","prompt":"not this"}}"#.to_owned() + "\n",
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Stopped."}]}}"#.to_owned() + "\n",
    ]
    .concat();
    let transcript = parse(lines.as_bytes());
    assert_eq!(
        texts(&transcript),
        vec![
            (Speaker::User, MessageKind::Text, "Tag the topics".to_owned()),
            (Speaker::Assistant, MessageKind::Tool, "Bash: ls".to_owned()),
            (Speaker::User, MessageKind::Text, "stop".to_owned()),
            (Speaker::User, MessageKind::Text, "and say why".to_owned()),
            (Speaker::Assistant, MessageKind::Text, "Stopped.".to_owned()),
        ]
    );
    assert_eq!(transcript.messages[2].ts_ms, 1_792_939_080_000);
}

#[test]
fn a_shell_command_you_ran_shows_as_typed_with_its_output_in_one_row() {
    let lines = [
        user_line("<bash-input>git push origin a -> b</bash-input>"),
        user_line(
            "<bash-stdout>To https://github.com/x/y.git\n + 6599...6194 a -&gt; b (forced update)</bash-stdout><bash-stderr></bash-stderr>",
        ),
        user_line("<bash-input>false</bash-input>"),
        user_line("<bash-stdout></bash-stdout><bash-stderr>fatal: &#x27;x&#x27; isn&#x27;t a repo</bash-stderr>"),
        user_line("<bash-input>true</bash-input>"),
        user_line("<bash-stdout></bash-stdout><bash-stderr></bash-stderr>"),
    ]
    .concat();
    let transcript = parse(lines.as_bytes());
    assert_eq!(
        texts(&transcript),
        vec![
            (
                Speaker::User,
                MessageKind::Text,
                "! git push origin a -> b".to_owned()
            ),
            (
                Speaker::User,
                MessageKind::Tool,
                "shell_output: To https://github.com/x/y.git…".to_owned()
            ),
            (Speaker::User, MessageKind::Text, "! false".to_owned()),
            (
                Speaker::User,
                MessageKind::Tool,
                "shell_output: fatal: 'x' isn't a repo (error)".to_owned()
            ),
            (Speaker::User, MessageKind::Text, "! true".to_owned()),
            (
                Speaker::User,
                MessageKind::Tool,
                "shell_output: (no output)".to_owned()
            ),
        ]
    );
    let output = transcript.messages[1]
        .tool
        .as_ref()
        .and_then(|tool| tool.output.as_deref());
    assert_eq!(
        output,
        Some("To https://github.com/x/y.git\n + 6599...6194 a -> b (forced update)")
    );
    assert_eq!(transcript.messages[5].tool.as_ref().unwrap().output, None);
    // Every row is final: nothing waits for a result.
    assert_eq!(transcript.next, 6);
}

fn user_line(text: &str) -> String {
    serde_json::json!({"type": "user", "message": {"role": "user", "content": text}}).to_string() + "\n"
}

#[test]
fn the_cache_parses_only_what_was_appended() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let cache = Cache::default();
    let first = user_line("first");
    std::fs::write(&path, &first).unwrap();
    assert_eq!(cache.read(&path, None).unwrap().messages.len(), 1);
    // Unchanged: the same answer from the cache.
    assert_eq!(cache.read(&path, None).unwrap(), parse(first.as_bytes()));

    // The file grows, and its first line is overwritten with a different one
    // of the same length: only the appended lines are read, so "first"
    // stays as it was parsed.
    let replaced = user_line("FIRST");
    assert_eq!(replaced.len(), first.len());
    let grown = format!("{replaced}{}", user_line("second"));
    std::fs::write(&path, &grown).unwrap();
    let texts = |transcript: &Transcript| -> Vec<String> {
        transcript
            .messages
            .iter()
            .map(|message| message.text.clone().unwrap_or_default())
            .collect()
    };
    assert_eq!(texts(&cache.read(&path, None).unwrap()), ["first", "second"]);

    // A line still being written waits, then counts once it's complete.
    let third = user_line("third");
    let (head, tail) = third.split_at(10);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(head.as_bytes())
        .unwrap();
    assert_eq!(cache.read(&path, None).unwrap().messages.len(), 2);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(tail.as_bytes())
        .unwrap();
    assert_eq!(
        texts(&cache.read(&path, None).unwrap()),
        ["first", "second", "third"]
    );
}

#[test]
fn the_cache_starts_over_when_the_file_shrinks_or_is_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let cache = Cache::default();
    std::fs::write(&path, format!("{}{}", user_line("one"), user_line("two"))).unwrap();
    assert_eq!(cache.read(&path, None).unwrap().messages.len(), 2);

    // Truncated: a new session in the same file.
    std::fs::write(&path, user_line("fresh")).unwrap();
    assert_eq!(
        cache.read(&path, None).unwrap(),
        parse(user_line("fresh").as_bytes())
    );

    // Rewritten longer, with the old last line's end no longer a newline:
    // parsed again from the top rather than glued on.
    let rewritten = format!("{}{}", user_line("a much longer opening line"), user_line("next"));
    std::fs::write(&path, &rewritten).unwrap();
    assert_eq!(cache.read(&path, None).unwrap(), parse(rewritten.as_bytes()));

    // Gone: empty, and forgotten.
    std::fs::remove_file(&path).unwrap();
    assert_eq!(cache.read(&path, None).unwrap(), Transcript::default());
    // Back with a context, read from the top.
    std::fs::write(&path, CONTEXT).unwrap();
    assert_eq!(cache.read(&path, None).unwrap(), parse(CONTEXT.as_bytes()));
}

#[test]
fn pastes_unwrap_and_keep_the_text_around_them() {
    assert_eq!(
        unwrap_pastes("before <pasted_content id=\"x\">\ninside\n</pasted_content id=\"x\"> after"),
        "before \ninside\n after"
    );
    assert_eq!(unwrap_pastes("no tags <here>"), "no tags <here>");
    assert_eq!(
        unwrap_pastes("broken <pasted_content id"),
        "broken <pasted_content id"
    );
}

#[test]
fn messages_serialize_in_the_contract_shape() {
    let transcript = parse(WORKER.as_bytes());
    let json = serde_json::to_value(&transcript).unwrap();
    assert_eq!(json["next"], 3);
    assert_eq!(
        json["messages"][1],
        serde_json::json!({
            "n": 2,
            "role": "assistant",
            "kind": "tool",
            "tool": {
                "name": "Bash",
                "summary": "mkdir -p notes && printf 'hello\\n' > notes/hello.txt && git add notes/hello.txt && git commit -m \"Add hello note\"",
                "is_error": false
            },
            "ts_ms": 1_792_620_654_442_i64
        })
    );
    assert_eq!(
        json["messages"][0]["text"]
            .as_str()
            .map(|text| text.starts_with("Task:")),
        Some(true)
    );
}

#[test]
fn cos_agent_tools_name_their_agent() {
    let assistant = |id: &str, name: &str, input: Value| {
        serde_json::json!({"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": id, "name": name, "input": input}
        ]}})
        .to_string()
            + "\n"
    };
    let result = |id: &str, content: Value, is_error: bool| {
        serde_json::json!({"type": "user", "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": id, "content": content, "is_error": is_error}
        ]}})
        .to_string()
            + "\n"
    };
    let lines = [
        assistant(
            "t1",
            "mcp__cos__agent_spawn",
            serde_json::json!({"prompt": "Fix the flaky test\nIt fails one run in three.", "new_workspace": {}}),
        ),
        result(
            "t1",
            serde_json::json!([{"type": "text", "text": "{\n  \"agent_id\": 17,\n  \"workspace_id\": 4\n}"}]),
            false,
        ),
        assistant(
            "t2",
            "mcp__cos__agent_spawn",
            serde_json::json!({"prompt": "Draft notes", "title": "Release notes", "workspace_id": 4}),
        ),
        result("t2", serde_json::json!("{\"agent_id\": 18, \"workspace_id\": 4}"), false),
        assistant("t3", "mcp__cos__agent_spawn", serde_json::json!({"prompt": "Oops"})),
        result("t3", serde_json::json!("give exactly one of workspace_id or new_workspace"), true),
        assistant("t4", "mcp__cos__agent_read", serde_json::json!({"agent_id": 17})),
        assistant("t5", "Bash", serde_json::json!({"command": "ls", "agent_id": 3})),
    ]
    .concat();
    let transcript = parse(lines.as_bytes());
    let calls: Vec<(&str, Option<i64>)> = transcript
        .messages
        .iter()
        .filter_map(|message| message.tool.as_ref())
        .map(|tool| (tool.summary.as_str(), tool.agent_id))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("Fix the flaky test…", Some(17)),
            ("Release notes", Some(18)),
            ("Oops", None),
            ("{\"agent_id\":17}", Some(17)),
            ("ls", None),
        ]
    );
    let json = serde_json::to_value(&transcript.messages[0]).unwrap();
    assert_eq!(json["tool"]["agent_id"], 17);
    let json = serde_json::to_value(&transcript.messages[4]).unwrap();
    assert!(json["tool"].get("agent_id").is_none());
}

#[test]
fn attachments_come_out_of_the_text() {
    let pasted = "<pasted_content id=\"1\">Fix this layout\n\n\
        Attached files (saved by cos; read them with the Read tool):\n\
        - /w/app/.cos/attachments/shot.png (image/png, 245 KB)</pasted_content id=\"1\">";
    let only = "Attached files (saved by cos; read them with the Read tool):\n\
        - /w/app/.cos/attachments/spec.pdf (application/pdf, 3.3 MB)";
    let transcript = parse((user_line(pasted) + &user_line(only)).as_bytes());
    let first = &transcript.messages[0];
    assert_eq!(first.text.as_deref(), Some("Fix this layout"));
    assert_eq!(first.attachments.len(), 1);
    assert_eq!(first.attachments[0].name, "shot.png");
    assert_eq!(first.attachments[0].path, "/w/app/.cos/attachments/shot.png");
    let second = &transcript.messages[1];
    assert_eq!(second.text.as_deref(), Some(""));
    assert_eq!(second.attachments[0].content_type, "application/pdf");

    let json = serde_json::to_value(first).unwrap();
    assert_eq!(
        json["attachments"],
        serde_json::json!([{
            "name": "shot.png",
            "path": "/w/app/.cos/attachments/shot.png",
            "content_type": "image/png",
            "size_label": "245 KB"
        }])
    );
    let plain = parse(user_line("hello").as_bytes());
    assert!(
        serde_json::to_value(&plain.messages[0])
            .unwrap()
            .get("attachments")
            .is_none()
    );
}
