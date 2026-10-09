use super::*;

/// `/sessions list` and `/sessions delete <id>` through the real headless
/// binary: the slash forms drive the same shared saved-session operations
/// `saya sessions` uses. The listing names a second saved session written
/// directly into the scratch store and ends with the storage root; a delete
/// exits zero, prints the retained-data sentence, removes exactly that
/// session's record and keeps the live session's own; a delete of an
/// unknown id exits non-zero saying so. stdin is null throughout — the
/// forms never prompt.
#[test]
fn turn_file_sessions_list_and_delete_drive_the_shared_operations() {
    const RETAINED_NOTE: &str = "Project preferences and typed database knowledge are stored \
                                 separately and were not deleted.";
    let env = test_root("sessions-slash");
    let (address, _bodies) = mock("unused");
    // A second saved session in the scratch store, as a minimal
    // legacy-shaped record the store accepts.
    let other = env.sessions.join("other-session.json");
    std::fs::write(&other, r#"{"id":"other-session","turns":[],"messages":[]}"#).unwrap();
    let run = |line: &str| {
        let path = env.root.join("instruction.md");
        std::fs::write(&path, format!("{line}\n")).unwrap();
        let mut args = base_args(&env);
        args.push("--turn-file".into());
        args.push(path.to_str().unwrap().into());
        saya(
            &env,
            &address,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .output()
        .unwrap()
    };

    // An unknown id: non-zero, the refusal names the id, nothing is removed.
    let output = run("/sessions delete nosuch-session-0001");
    assert_ne!(
        output.status.code(),
        Some(0),
        "an unknown-id delete must exit non-zero; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("no saved session nosuch-session-0001"),
        "the refusal names the id: {stderr}"
    );
    assert!(other.exists(), "an unknown-id delete removes nothing");

    // The listing: exit zero, the saved session is named, the storage root
    // is the last line.
    let output = run("/sessions list");
    assert_eq!(
        output.status.code(),
        Some(0),
        "the listing completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("other-session"),
        "the saved session is listed: {stdout}"
    );
    assert!(
        stdout.contains("Storage root:"),
        "the listing ends with the storage root: {stdout}"
    );

    // The delete: exit zero, the retained-data sentence, the record gone,
    // the live session's own record kept.
    let output = run("/sessions delete other-session");
    assert_eq!(
        output.status.code(),
        Some(0),
        "the delete completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("Deleted saved session other-session"),
        "the success names the id: {stdout}"
    );
    assert!(
        stdout.contains(RETAINED_NOTE),
        "the retained-data sentence is said: {stdout}"
    );
    assert!(!other.exists(), "the deleted session's record is gone");
    let remaining: Vec<String> = std::fs::read_dir(&env.sessions)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    assert!(
        !remaining.is_empty() && remaining.iter().all(|name| name != "other-session.json"),
        "the current session's own record survives and no other-session record remains: \
         {remaining:?}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
