/// `/sessions list` through the shared builder: the same listing the
/// headless `saya sessions list` renders — rows, and the storage root
/// as the last line; an empty store says "No saved sessions.".
#[test]
fn sessions_list_text_renders_the_shared_listing() {
    let root = scratch("list");
    let store = FsSessionStore::new(&root);
    block_on(store.save(RedactedSession {
        id: "other".into(),
        messages: vec![RedactedMessage {
            role: "user".into(),
            content: "hi".into(),
        }],
        ..Default::default()
    }))
    .unwrap();
    let text = sessions_list_text(&store, &root).expect("the listing renders");
    assert!(
        text.lines().any(|line| line.starts_with("other\t")),
        "the saved session is a row: {text}"
    );
    assert!(
        text.ends_with(&format!("Storage root: {}", root.display())),
        "the root is the last line: {text}"
    );
    let empty = scratch("list-empty");
    let empty_store = FsSessionStore::new(&empty);
    let text = sessions_list_text(&empty_store, &empty).expect("the empty listing renders");
    assert!(
        text.starts_with("No saved sessions."),
        "an empty store says so: {text}"
    );
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&empty);
}

/// `/sessions delete` through the shared builder: a real delete says
/// the retained-data sentence and removes the record; the current
/// session is refused with the D09a words and nothing is touched; an
/// unknown id says "no saved session".
#[test]
fn sessions_delete_text_reports_success_refusals_and_unknown_ids() {
    let root = scratch("delete");
    std::fs::write(root.join("a.json"), "{}").unwrap();
    std::fs::create_dir_all(root.join("a")).unwrap();
    let store = FsSessionStore::new(&root);
    let (ok, text) = sessions_delete_text(&store, &root, "a", Some("a"));
    assert!(!ok, "the current session is refused: {ok}");
    assert!(
        text.contains("live session") && text.contains("was not deleted"),
        "the refusal is the current-session one: {text}"
    );
    assert!(root.join("a.json").exists(), "the record survived");
    let (ok, text) = sessions_delete_text(&store, &root, "a", None);
    assert!(ok, "a foreign delete completes: {text}");
    assert!(
        text.contains("Deleted saved session a") && text.contains("were not deleted"),
        "the success carries the retained-data sentence: {text}"
    );
    assert!(!root.join("a.json").exists(), "the record is gone");
    let (ok, text) = sessions_delete_text(&store, &root, "nosuch-id", None);
    assert!(!ok, "an unknown id is refused: {text}");
    assert!(
        text.contains("no saved session nosuch-id"),
        "the unknown-id refusal names the id: {text}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
