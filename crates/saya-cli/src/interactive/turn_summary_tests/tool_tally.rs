use super::*;

/// Requests and completions pair by order and name: a completed event with
/// no pending request is ignored entirely; a requested call with no
/// completion is listed; a file-writing call's entry carries name + path and
/// NEVER the call's content; a non-file tool is named alone; a denied call
/// settled (its effect is known: none); same-name nesting pairs oldest
/// first.
#[test]
fn the_tally_pairs_requests_and_completions_and_lists_unfinished_calls() {
    let counted = tally(&[
        requested("workspace_read", serde_json::json!({"path": "a.txt"})),
        completed("workspace_read"),
        requested("workspace_edit", serde_json::json!({"path": "b.txt"})),
    ]);
    assert_eq!(
        counted.completed(),
        1,
        "the read completed; the edit did not"
    );
    assert_eq!(
        counted.unfinished_entries(),
        vec!["workspace_edit b.txt".to_owned()]
    );
    let orphan = tally(&[completed("ghost")]);
    assert_eq!(orphan.completed(), 0, "an unmatched completion is ignored");
    assert!(orphan.unfinished_entries().is_empty());
    let content = tally(&[requested(
        "workspace_write",
        serde_json::json!({"path": "c.txt", "content": "SECRET-FILE-CONTENT"}),
    )]);
    let entries = content.unfinished_entries();
    assert_eq!(entries, vec!["workspace_write c.txt".to_owned()]);
    assert!(!entries[0].contains("SECRET-FILE-CONTENT"));
    let sql = tally(&[requested(
        "bounded_sql_query",
        serde_json::json!({"sql": "SELECT token FROM secrets"}),
    )]);
    assert_eq!(
        sql.unfinished_entries(),
        vec!["bounded_sql_query".to_owned()],
        "a non-file tool is named alone"
    );
    let refused = tally(&[
        requested("workspace_write", serde_json::json!({"path": "d.txt"})),
        denied("workspace_write"),
    ]);
    assert_eq!(refused.completed(), 0);
    assert!(
        refused.unfinished_entries().is_empty(),
        "a denied call's effect is known: none"
    );
    let nested = tally(&[
        requested("workspace_read", serde_json::json!({})),
        requested("workspace_read", serde_json::json!({})),
        completed("workspace_read"),
    ]);
    assert_eq!(nested.completed(), 1);
    assert_eq!(
        nested.unfinished_entries(),
        vec!["workspace_read".to_owned()]
    );
}
