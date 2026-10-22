use std::path::Path;

use serde_json::json;

use super::*;

fn setup() -> (Arc<Store>, EventLog, Environment) {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let log = EventLog::new(store.clone());
    let env = Environment::new(store.clone(), log.clone());
    (store, log, env)
}

fn add_project(store: &Store, id: i64) {
    store
        .lock()
        .execute(
            "INSERT INTO projects (id, root, name, base_branch, created_ms) VALUES (?1, ?2, 'p', 'main', 0)",
            params![id, format!("/repo/{id}")],
        )
        .unwrap();
}

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn the_project_list_wins_over_the_global_one() {
    let (store, _, env) = setup();
    add_project(&store, 1);
    add_project(&store, 2);
    env.set(None, "CLAUDE_CONFIG_DIR", "~/.claude").unwrap();
    env.set(None, "SHARED", "global").unwrap();
    env.set(Some(1), "CLAUDE_CONFIG_DIR", "~/.claude-personal")
        .unwrap();
    env.set(Some(1), "ONLY_ONE", "x").unwrap();

    let one = env.project(1).unwrap();
    assert_eq!(
        one.variables,
        vars(&[("CLAUDE_CONFIG_DIR", "~/.claude-personal"), ("ONLY_ONE", "x")])
    );
    assert_eq!(
        one.global,
        vars(&[("CLAUDE_CONFIG_DIR", "~/.claude"), ("SHARED", "global")])
    );
    assert_eq!(
        one.effective,
        vars(&[
            ("CLAUDE_CONFIG_DIR", "~/.claude-personal"),
            ("ONLY_ONE", "x"),
            ("SHARED", "global")
        ])
    );
    assert_eq!(env.effective(1).unwrap(), one.effective);
    assert_eq!(env.effective(2).unwrap(), one.global);
    assert_eq!(env.global().unwrap(), one.global);
}

#[test]
fn setting_again_replaces_and_global_names_stay_unique() {
    let (store, _, env) = setup();
    add_project(&store, 1);
    env.set(None, "A", "1").unwrap();
    env.set(None, "A", "2").unwrap();
    env.set(Some(1), "A", "3").unwrap();
    env.set(Some(1), "A", "4").unwrap();
    assert_eq!(env.global().unwrap(), vars(&[("A", "2")]));
    assert_eq!(env.project(1).unwrap().variables, vars(&[("A", "4")]));
    let rows: i64 = store
        .lock()
        .query_row("SELECT COUNT(*) FROM env", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 2);
    // The index refuses a second global row even if something bypasses `set`.
    let duplicate = store.lock().execute(
        "INSERT INTO env (project_id, name, value, updated_ms) VALUES (NULL, 'A', 'x', 0)",
        [],
    );
    assert!(duplicate.is_err());
}

#[test]
fn unset_says_whether_there_was_one() {
    let (store, _, env) = setup();
    add_project(&store, 1);
    env.set(Some(1), "A", "1").unwrap();
    assert!(env.unset(Some(1), "A").unwrap());
    assert!(!env.unset(Some(1), "A").unwrap());
    assert!(!env.unset(None, "A").unwrap());
    assert!(env.project(1).unwrap().variables.is_empty());
}

#[test]
fn unknown_projects_are_not_found() {
    let (_, _, env) = setup();
    assert!(matches!(env.project(9), Err(EnvError::NotFound(_))));
    assert!(matches!(env.set(Some(9), "A", "1"), Err(EnvError::NotFound(_))));
    assert!(matches!(env.unset(Some(9), "A"), Err(EnvError::NotFound(_))));
}

#[test]
fn events_name_the_variable_but_never_hold_its_value() {
    let (store, log, env) = setup();
    add_project(&store, 1);
    env.set(None, "API_KEY", "sk-secret-value").unwrap();
    env.set(Some(1), "API_KEY", "sk-other-secret").unwrap();
    env.unset(Some(1), "API_KEY").unwrap();
    // Nothing to remove: nothing logged.
    env.unset(Some(1), "API_KEY").unwrap();

    let events = log.after(0, 100).unwrap();
    let data: Vec<_> = events
        .iter()
        .map(|event| (event.kind.as_str(), event.data.clone()))
        .collect();
    assert_eq!(
        data,
        [
            (
                kind::ENV_CHANGED,
                json!({"project_id": null, "name": "API_KEY", "action": "set"})
            ),
            (
                kind::ENV_CHANGED,
                json!({"project_id": 1, "name": "API_KEY", "action": "set"})
            ),
            (
                kind::ENV_CHANGED,
                json!({"project_id": 1, "name": "API_KEY", "action": "removed"})
            ),
        ]
    );
    for event in &events {
        assert!(!event.data.to_string().contains("secret"), "{event:?}");
    }
}

#[test]
fn removing_a_project_removes_its_variables() {
    let (store, _, env) = setup();
    add_project(&store, 1);
    env.set(None, "KEEP", "1").unwrap();
    env.set(Some(1), "GONE", "1").unwrap();
    store.delete_project(1).unwrap();
    let names: Vec<String> = store
        .lock()
        .prepare("SELECT name FROM env")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(names, ["KEEP"]);
}

#[test]
fn names_and_values_are_checked() {
    let longest = "N".repeat(MAX_NAME_CHARS);
    for good in ["A", "_", "a_b", "PATH", "X1", &longest] {
        assert!(check_name(good).is_ok(), "{good}");
    }
    let too_long = "N".repeat(MAX_NAME_CHARS + 1);
    for bad in ["", "1A", "A-B", "A B", "A=B", "É", &too_long] {
        assert!(check_name(bad).is_err(), "{bad}");
    }
    assert!(check_value("A", "").is_ok());
    assert!(check_value("A", &"v".repeat(MAX_VALUE_BYTES)).is_ok());
    let long = check_value("A", &"secret".repeat(MAX_VALUE_BYTES)).unwrap_err();
    assert!(!long.contains("secret"), "{long}");
    let nul = check_value("A", "sec\0ret").unwrap_err();
    assert!(!nul.contains("sec\0"), "{nul}");

    let (_, _, env) = setup();
    assert!(matches!(env.set(None, "1BAD", "x"), Err(EnvError::Invalid(_))));
    assert!(matches!(env.set(None, "A", "a\0b"), Err(EnvError::Invalid(_))));
    assert!(env.global().unwrap().is_empty());
}

#[test]
fn a_leading_tilde_becomes_the_home_folder() {
    let home = Some(Path::new("/home/me"));
    assert_eq!(expand_home("~", home), "/home/me");
    assert_eq!(
        expand_home("~/.claude-personal", home),
        "/home/me/.claude-personal"
    );
    for unchanged in ["", "/abs/~/x", "~other/x", "a~/b", "~~", "sk-~/x"] {
        assert_eq!(expand_home(unchanged, home), unchanged);
    }
    assert_eq!(expand_home("~/x", None), "~/x");
}
