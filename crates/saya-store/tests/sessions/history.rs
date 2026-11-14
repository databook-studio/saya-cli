use super::*;

#[test]
fn history_lists_valid_sessions_in_recent_first_order() {
    let root = temp_root("history");
    let store = FsSessionStore::new(&root);
    block_on(store.save(RedactedSession {
        id: "older".into(),
        profile_names: vec![],
        messages: vec![],
        ..Default::default()
    }))
    .unwrap();
    let epoch = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    std::fs::File::options()
        .write(true)
        .open(root.join("older.json"))
        .unwrap()
        .set_times(
            std::fs::FileTimes::new().set_modified(epoch + std::time::Duration::from_micros(100)),
        )
        .unwrap();
    block_on(store.save(RedactedSession {
        id: "newer".into(),
        profile_names: vec![],
        messages: vec![],
        ..Default::default()
    }))
    .unwrap();
    std::fs::File::options()
        .write(true)
        .open(root.join("newer.json"))
        .unwrap()
        .set_times(
            std::fs::FileTimes::new().set_modified(epoch + std::time::Duration::from_micros(200)),
        )
        .unwrap();
    let history = block_on(store.history(SessionHistoryQuery::first_page(2).unwrap())).unwrap();
    assert_eq!(history.entries.len(), 2);
    assert_eq!(history.entries[0].id, "newer");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn history_pages_are_bounded_and_continue_without_duplicates() {
    let root = temp_root("history_pages");
    let _ = std::fs::remove_dir_all(&root);
    let store = FsSessionStore::new(&root);
    for index in 0..7 {
        block_on(store.save(RedactedSession {
            id: format!("session-{index:02}"),
            profile_names: vec![],
            messages: vec![],
            ..Default::default()
        }))
        .unwrap();
    }

    let query = SessionHistoryQuery::first_page(3).unwrap();
    let first = block_on(store.history(query.clone())).unwrap();
    assert_eq!(first.entries.len(), 3);
    assert!(first.next_cursor.is_some());
    let second_query = query.next_page(&first).expect("first page continues");
    let second = block_on(store.history(second_query.clone())).unwrap();
    let third_query = second_query
        .next_page(&second)
        .expect("second page continues");
    let third = block_on(store.history(third_query)).unwrap();

    let ids = first
        .entries
        .into_iter()
        .chain(second.entries)
        .chain(third.entries)
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    let unique = ids.iter().collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), 7);
    assert_eq!(unique.len(), ids.len());
    assert!(third.next_cursor.is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn history_page_size_is_typed_and_bounded() {
    assert!(SessionHistoryQuery::first_page(0).is_err());
    assert!(
        SessionHistoryQuery::first_page(saya_store::MAX_SESSION_HISTORY_PAGE_SIZE + 1).is_err()
    );
}
