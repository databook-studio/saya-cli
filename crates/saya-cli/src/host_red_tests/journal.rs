//! The allow-side journal event for host commands.

/// Every host call is journalled before the child spawns.
#[test]
fn every_host_call_is_journalled_before_the_child_spawns() {
    let dir = std::env::temp_dir().join(format!("saya-h2-journal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("state dir creates");
    let journal = saya_store::SessionJournal::open(&dir);
    journal
        .command("npm", &["install".to_owned()], "run_command")
        .expect("the allow-side event journals");
    let events = journal.read().expect("the journal reads");
    assert_eq!(
        events,
        vec![saya_store::JournalEvent::Command {
            program: "npm".to_owned(),
            argv: vec!["install".to_owned()],
            door: "run_command".to_owned(),
        }],
        "shape: the program, its argv, and the door"
    );
    let raw = std::fs::read_to_string(dir.join("journal.ndjson")).expect("the journal reads");
    assert!(
        raw.contains("session-command") && !raw.contains("session-command-denied"),
        "the allow side writes its own kind: {raw}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
