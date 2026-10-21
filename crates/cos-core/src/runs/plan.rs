//! Plans the chief proposes: a list of tasks with their file scopes and
//! dependencies. [`validate`] checks one before it reaches the board.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::taskgit;

/// The most tasks one plan may have.
pub const MAX_TASKS: usize = 12;
const MAX_TITLE_CHARS: usize = 200;

/// What the chief sends with `board_propose`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanProposal {
    pub tasks: Vec<ProposedTask>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposedTask {
    /// Short and unique within the plan. Other tasks name it in `depends_on`.
    pub title: String,
    /// What the worker is told to do.
    #[serde(alias = "description")]
    pub instructions: String,
    /// The files the worker may change: paths or globs relative to the repo,
    /// such as `src/limit.rs` or `src/limit/**`.
    pub files: Vec<String>,
    /// Run after the worker finishes; the task only goes to review if it passes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_command: Option<String>,
    /// Titles of tasks in the same plan that must be merged first.
    #[serde(default)]
    pub depends_on: Vec<String>,
}

/// A proposed task after validation: trimmed, with dependencies as indexes
/// into the plan.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheckedTask {
    pub title: String,
    pub instructions: String,
    pub files: Vec<String>,
    pub test_command: Option<String>,
    pub depends_on: Vec<usize>,
}

/// Checks a plan. On failure, the message lists every problem, so the chief
/// can fix them all in one go.
pub(crate) fn validate(plan: &PlanProposal) -> Result<Vec<CheckedTask>, String> {
    let mut problems = Vec::new();
    if plan.tasks.is_empty() {
        return Err("a plan needs at least one task".into());
    }
    if plan.tasks.len() > MAX_TASKS {
        problems.push(format!(
            "a plan can have at most {MAX_TASKS} tasks, this one has {}",
            plan.tasks.len()
        ));
    }

    let mut by_title: HashMap<String, usize> = HashMap::new();
    for (index, task) in plan.tasks.iter().enumerate() {
        let title = task.title.trim();
        if title.is_empty() {
            problems.push(format!("task {} has no title", index + 1));
        } else if title.chars().count() > MAX_TITLE_CHARS {
            problems.push(format!(
                "the title of task {} is over {MAX_TITLE_CHARS} characters",
                index + 1
            ));
        } else if by_title.insert(title.to_lowercase(), index).is_some() {
            problems.push(format!("more than one task is called \"{title}\""));
        }
    }

    let mut tasks = Vec::with_capacity(plan.tasks.len());
    for (index, task) in plan.tasks.iter().enumerate() {
        let name = describe(task, index);
        if task.instructions.trim().is_empty() {
            problems.push(format!("{name} has no instructions"));
        }
        let files = check_files(&task.files, &name, &mut problems);
        let mut depends_on = Vec::new();
        for dependency in &task.depends_on {
            match by_title.get(&dependency.trim().to_lowercase()) {
                Some(&other) if other == index => problems.push(format!("{name} depends on itself")),
                Some(&other) => {
                    if !depends_on.contains(&other) {
                        depends_on.push(other);
                    }
                }
                None => problems.push(format!(
                    "{name} depends on \"{}\", which isn't in the plan",
                    dependency.trim()
                )),
            }
        }
        tasks.push(CheckedTask {
            title: task.title.trim().to_owned(),
            instructions: task.instructions.trim().to_owned(),
            files,
            test_command: task
                .test_command
                .as_deref()
                .map(str::trim)
                .filter(|command| !command.is_empty())
                .map(str::to_owned),
            depends_on,
        });
    }

    let deps: Vec<Vec<usize>> = tasks.iter().map(|task| task.depends_on.clone()).collect();
    if let Some(cycle) = find_cycle(&deps) {
        let names: Vec<String> = cycle
            .iter()
            .map(|&index| format!("\"{}\"", tasks[index].title))
            .collect();
        problems.push(format!(
            "the dependencies go round in a circle: {}",
            names.join(" → ")
        ));
    } else {
        problems.extend(shared_files(&tasks, &deps));
    }

    if problems.is_empty() {
        Ok(tasks)
    } else {
        Err(problems.join("; "))
    }
}

fn describe(task: &ProposedTask, index: usize) -> String {
    let title = task.title.trim();
    if title.is_empty() {
        format!("task {}", index + 1)
    } else {
        format!("\"{title}\"")
    }
}

/// Trims and de-duplicates a task's file patterns. Each must be relative to
/// the repo, stay inside it, avoid protected paths (which the scope check
/// after the worker always refuses) and be a pattern the scope check reads.
fn check_files(files: &[String], name: &str, problems: &mut Vec<String>) -> Vec<String> {
    let mut checked: Vec<String> = Vec::new();
    let mut refused = false;
    for pattern in files {
        let pattern = pattern.trim().trim_start_matches("./");
        if pattern.is_empty() {
            continue;
        }
        let problem = if pattern.starts_with('/') || pattern.starts_with('~') {
            Some(format!("{name}: {pattern} must be relative to the repo"))
        } else if pattern.split('/').any(|part| part == "..") {
            Some(format!("{name}: {pattern} leaves the repo"))
        } else if taskgit::is_protected(pattern) {
            Some(format!("{name}: {pattern} is protected; no task may change it"))
        } else {
            taskgit::Scope::new([pattern])
                .err()
                .map(|error| format!("{name}: {error}"))
        };
        if let Some(problem) = problem {
            problems.push(problem);
            refused = true;
        } else if !checked.iter().any(|seen| seen == pattern) {
            checked.push(pattern.to_owned());
        }
    }
    if checked.is_empty() && !refused {
        problems.push(format!("{name} lists no files; every task needs a file scope"));
    }
    checked
}

/// A dependency cycle as task indexes, first task repeated at the end, if
/// there is one.
fn find_cycle(deps: &[Vec<usize>]) -> Option<Vec<usize>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        OnPath,
        Done,
    }

    fn visit(
        node: usize,
        deps: &[Vec<usize>],
        marks: &mut [Mark],
        path: &mut Vec<usize>,
    ) -> Option<Vec<usize>> {
        marks[node] = Mark::OnPath;
        path.push(node);
        for &next in &deps[node] {
            match marks[next] {
                Mark::OnPath => {
                    let start = path.iter().position(|&n| n == next).expect("on the path");
                    let mut cycle = path[start..].to_vec();
                    cycle.push(next);
                    return Some(cycle);
                }
                Mark::New => {
                    if let Some(cycle) = visit(next, deps, marks, path) {
                        return Some(cycle);
                    }
                }
                Mark::Done => {}
            }
        }
        path.pop();
        marks[node] = Mark::Done;
        None
    }

    let mut marks = vec![Mark::New; deps.len()];
    (0..deps.len()).find_map(|node| {
        (marks[node] == Mark::New)
            .then(|| visit(node, deps, &mut marks, &mut Vec::new()))
            .flatten()
    })
}

/// Two tasks that can run at the same time must not list the same file.
/// Tasks ordered by a dependency may, since the later one starts from the
/// earlier one's merge. Only identical patterns are caught here; the diff
/// check after each worker is what actually enforces the scope.
fn shared_files(tasks: &[CheckedTask], deps: &[Vec<usize>]) -> Vec<String> {
    let before = ancestors(deps);
    let mut problems = Vec::new();
    let mut reported = HashSet::new();
    for (a, first) in tasks.iter().enumerate() {
        for (b, second) in tasks.iter().enumerate().skip(a + 1) {
            if before[a].contains(&b) || before[b].contains(&a) {
                continue;
            }
            for file in &first.files {
                if second.files.contains(file) && reported.insert((a, b)) {
                    problems.push(format!(
                        "\"{}\" and \"{}\" both list {file} but can run at the same time; \
                         give the file to one of them or make one depend on the other",
                        first.title, second.title
                    ));
                }
            }
        }
    }
    problems
}

/// For each task, every task it depends on directly or indirectly. Expects
/// no cycles.
fn ancestors(deps: &[Vec<usize>]) -> Vec<HashSet<usize>> {
    fn fill(node: usize, deps: &[Vec<usize>], memo: &mut [Option<HashSet<usize>>]) -> HashSet<usize> {
        if let Some(known) = &memo[node] {
            return known.clone();
        }
        let mut all = HashSet::new();
        for &dep in &deps[node] {
            all.insert(dep);
            all.extend(fill(dep, deps, memo));
        }
        memo[node] = Some(all.clone());
        all
    }

    let mut memo = vec![None; deps.len()];
    (0..deps.len()).map(|node| fill(node, deps, &mut memo)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(title: &str, files: &[&str], depends_on: &[&str]) -> ProposedTask {
        ProposedTask {
            title: title.into(),
            instructions: format!("do {title}"),
            files: files.iter().map(|f| (*f).to_owned()).collect(),
            test_command: None,
            depends_on: depends_on.iter().map(|d| (*d).to_owned()).collect(),
        }
    }

    fn plan(tasks: Vec<ProposedTask>) -> PlanProposal {
        PlanProposal { tasks }
    }

    #[test]
    fn a_good_plan_is_trimmed_and_resolved() {
        let mut api = task("  API ", &["./src/api.rs", "src/api.rs", " "], &[]);
        api.test_command = Some("  cargo test  ".into());
        let mut ui = task("UI", &["src/ui/**"], &["api", "API"]);
        ui.test_command = Some("   ".into());

        let tasks = validate(&plan(vec![api, ui])).unwrap();

        assert_eq!(tasks[0].title, "API");
        assert_eq!(tasks[0].files, ["src/api.rs"]);
        assert_eq!(tasks[0].test_command.as_deref(), Some("cargo test"));
        assert_eq!(tasks[1].depends_on, [0]);
        assert_eq!(tasks[1].test_command, None);
    }

    #[test]
    fn empty_and_oversized_plans_are_refused() {
        assert!(validate(&plan(vec![])).unwrap_err().contains("at least one task"));

        let many = (0..=MAX_TASKS)
            .map(|n| task(&format!("t{n}"), &[&format!("f{n}")], &[]))
            .collect();
        let error = validate(&plan(many)).unwrap_err();
        assert!(error.contains("at most 12 tasks"), "{error}");
    }

    #[test]
    fn titles_must_be_present_and_unique() {
        let error = validate(&plan(vec![
            task("Same", &["a"], &[]),
            task("same", &["b"], &[]),
            task(" ", &["c"], &[]),
        ]))
        .unwrap_err();

        assert!(error.contains("more than one task is called \"same\""), "{error}");
        assert!(error.contains("task 3 has no title"), "{error}");
    }

    #[test]
    fn dependencies_must_exist_and_not_be_self() {
        let error = validate(&plan(vec![
            task("a", &["a"], &["ghost"]),
            task("b", &["b"], &["b"]),
        ]))
        .unwrap_err();

        assert!(
            error.contains("\"a\" depends on \"ghost\", which isn't in the plan"),
            "{error}"
        );
        assert!(error.contains("\"b\" depends on itself"), "{error}");
    }

    #[test]
    fn cycles_are_refused_and_named() {
        let error = validate(&plan(vec![
            task("a", &["a"], &["c"]),
            task("b", &["b"], &["a"]),
            task("c", &["c"], &["b"]),
            task("d", &["d"], &[]),
        ]))
        .unwrap_err();

        assert!(error.contains("circle: \"a\" → \"c\" → \"b\" → \"a\""), "{error}");
    }

    #[test]
    fn every_task_needs_a_file_scope_inside_the_repo() {
        let error = validate(&plan(vec![
            task("none", &[], &[]),
            task("blank", &["  "], &[]),
            task("absolute", &["/etc/passwd"], &[]),
            task("escape", &["src/../../x"], &[]),
            task("settings", &["src/a.rs", ".claude/settings.json"], &[]),
        ]))
        .unwrap_err();

        assert!(error.contains(".claude/settings.json is protected"), "{error}");

        assert!(error.contains("\"none\" lists no files"), "{error}");
        assert!(error.contains("\"blank\" lists no files"), "{error}");
        assert!(error.contains("/etc/passwd must be relative"), "{error}");
        assert!(error.contains("src/../../x leaves the repo"), "{error}");
    }

    #[test]
    fn parallel_tasks_may_not_share_a_file_but_ordered_ones_may() {
        let error = validate(&plan(vec![
            task("a", &["src/lib.rs"], &[]),
            task("b", &["src/lib.rs", "b"], &[]),
        ]))
        .unwrap_err();
        assert!(error.contains("\"a\" and \"b\" both list src/lib.rs"), "{error}");

        // c depends on a through b, so they never run at the same time.
        validate(&plan(vec![
            task("a", &["src/lib.rs"], &[]),
            task("b", &["b"], &["a"]),
            task("c", &["src/lib.rs"], &["b"]),
        ]))
        .unwrap();
    }

    #[test]
    fn description_is_accepted_for_instructions() {
        let proposal: PlanProposal = serde_json::from_value(serde_json::json!({
            "tasks": [{ "title": "a", "description": "do it", "files": ["a"] }]
        }))
        .unwrap();

        assert_eq!(proposal.tasks[0].instructions, "do it");
        assert!(proposal.tasks[0].depends_on.is_empty());
    }
}
