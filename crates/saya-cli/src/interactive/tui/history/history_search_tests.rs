use super::*;

/// Entries are set directly rather than pushed: `push` calls `save`, and
/// these tests only exercise `search`. Driving them through `push` wrote a
/// `saya-test-history` file into the crate directory on every `cargo test`.
fn history() -> History {
    History {
        entries: vec![
            "SELECT * FROM orders".to_string(),
            "explain select 1".to_string(),
            "select count(*) from events".to_string(),
        ],
        cursor: None,
        stash: None,
        path: std::path::PathBuf::new(),
        limit: MAX_ENTRIES,
        disabled: true,
        omitted: 0,
    }
}

#[test]
fn search_is_case_insensitive_and_newest_first() {
    let matches = history().search("SELECT");
    assert_eq!(
        matches,
        vec![
            "select count(*) from events",
            "explain select 1",
            "SELECT * FROM orders"
        ]
    );
    assert!(history().search("zzz").is_empty());
}
