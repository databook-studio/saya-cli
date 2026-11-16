#[test]
fn listing_labels_ok_and_near_limit_at_the_warning_band() {
    let sessions = scratch("labels");
    let base = sessions.parent().unwrap().to_path_buf();
    let store = FsSessionStore::new(&sessions);
    let small = RedactedSession {
        id: "small".into(),
        messages: vec![RedactedMessage {
            role: "user".into(),
            content: "hi".into(),
        }],
        ..Default::default()
    };
    let big = RedactedSession {
        id: "big".into(),
        messages: vec![RedactedMessage {
            role: "user".into(),
            content: "x".repeat(SESSION_WARN_BYTES),
        }],
        ..Default::default()
    };
    block_on(store.save(small)).unwrap();
    block_on(store.save(big)).unwrap();
    let listing = block_on(list_sessions(&store)).unwrap();
    assert_eq!(
        listing.rows.len(),
        2,
        "{:?}",
        listing
            .rows
            .iter()
            .map(|row| (&row.id, row.size_bytes, row.status))
            .collect::<Vec<_>>()
    );
    // The oversized record sits at or past the store's warning band, the
    // small one below it — both still listed, each labelled from the
    // store's own capacity classification.
    let big_row = listing.rows.iter().find(|row| row.id == "big").unwrap();
    let small_row = listing.rows.iter().find(|row| row.id == "small").unwrap();
    assert_eq!(big_row.status, SessionCapacityStatus::Warning);
    assert!(big_row.size_bytes >= SESSION_WARN_BYTES);
    assert_eq!(small_row.status, SessionCapacityStatus::Ok);
    let text = render_listing(&listing, super::operations::unix_ms_now(), &sessions);
    let big_line = text.lines().find(|line| line.starts_with("big\t")).unwrap();
    let small_line = text
        .lines()
        .find(|line| line.starts_with("small\t"))
        .unwrap();
    assert!(big_line.contains("near-limit"), "{big_line}");
    assert!(small_line.contains("ok"), "{small_line}");
    assert!(small_line.contains("KiB"), "{small_line}");
    assert!(text.ends_with(&format!("Storage root: {}", sessions.display())));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn listing_rows_are_newest_first_with_ages() {
    let sessions = scratch("order");
    let base = sessions.parent().unwrap().to_path_buf();
    std::fs::write(sessions.join("older.json"), "{}").unwrap();
    std::fs::write(sessions.join("newer.json"), "{}").unwrap();
    pinned_mtime(&sessions.join("older.json"), Duration::from_secs(30));
    let store = FsSessionStore::new(&sessions);
    let listing = block_on(list_sessions(&store)).unwrap();
    assert_eq!(
        listing
            .rows
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        vec!["newer", "older"]
    );
    // Ages come from the modification stamp at render time: a 90-second
    // age renders as one minute.
    let newest_ms = listing.rows[0].modified_unix_ms;
    let text = render_listing(&listing, newest_ms + 90_000, &sessions);
    let newest_line = text
        .lines()
        .find(|line| line.starts_with("newer\t"))
        .unwrap();
    assert!(newest_line.ends_with("1m ago"), "{newest_line}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn pending_deletions_are_listed_separately() {
    let sessions = scratch("pending");
    let base = sessions.parent().unwrap().to_path_buf();
    std::fs::write(sessions.join("a.json"), "{}").unwrap();
    // A pending-deletion tombstone lives outside the sessions tree (the
    // store's rule): `<sessions>.parent()/lifecycle/session-<id>.tombstone`.
    let lifecycle = base.join("lifecycle");
    std::fs::create_dir_all(&lifecycle).unwrap();
    std::fs::write(lifecycle.join("session-b.tombstone"), "{}").unwrap();
    let store = FsSessionStore::new(&sessions);
    let listing = block_on(list_sessions(&store)).unwrap();
    assert_eq!(listing.pending, vec!["b".to_owned()]);
    assert!(
        listing.rows.iter().all(|row| row.id != "b"),
        "a tombstoned session is hidden from the rows"
    );
    let text = render_listing(&listing, 0, &sessions);
    assert!(text.contains("pending deletion: b"), "{text}");
    assert!(!text.contains("pending deletion: a"), "{text}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn an_empty_directory_lists_no_saved_sessions() {
    let sessions = scratch("empty");
    let base = sessions.parent().unwrap().to_path_buf();
    let store = FsSessionStore::new(&sessions);
    let listing = block_on(list_sessions(&store)).unwrap();
    assert!(listing.rows.is_empty());
    assert!(listing.pending.is_empty());
    assert!(!listing.more);
    let text = render_listing(&listing, 0, &sessions);
    assert!(text.starts_with("No saved sessions."), "{text}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn listing_pages_up_to_the_bound_and_says_more_beyond() {
    let sessions = scratch("bound");
    let base = sessions.parent().unwrap().to_path_buf();
    for index in 0..1001 {
        std::fs::write(sessions.join(format!("s{index:04}.json")), "{}").unwrap();
    }
    let store = FsSessionStore::new(&sessions);
    let listing = block_on(list_sessions(&store)).unwrap();
    assert_eq!(listing.rows.len(), LISTING_BOUND);
    assert!(listing.more, "1001 sessions must say more exist");
    let text = render_listing(&listing, 0, &sessions);
    assert!(text.contains("(more available)"), "{text}");
    let _ = std::fs::remove_dir_all(base);
}
