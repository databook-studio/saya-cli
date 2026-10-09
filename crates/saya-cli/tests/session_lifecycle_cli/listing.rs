use super::support::*;
use std::time::Duration;

/// An empty sessions dir lists nothing, exits 0, and names the root.
#[test]
fn listing_an_empty_store_prints_no_saved_sessions() {
    let env = test_env("empty");
    let output = run(&env, &["sessions", "list"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        stdout(&output),
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(out.contains("No saved sessions."), "{out}");
    let _ = std::fs::remove_dir_all(&env.root);
}

/// Two saved sessions list newest first with a size, and the last line
/// names the storage root.
#[test]
fn listing_shows_saved_session_ids_sizes_and_the_storage_root() {
    let env = test_env("two");
    std::fs::write(env.sessions.join("older.json"), "{}").unwrap();
    std::fs::write(env.sessions.join("newer.json"), "{}").unwrap();
    age(&env.sessions.join("older.json"), Duration::from_secs(30));
    let output = run(&env, &["sessions", "list"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        stdout(&output),
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(out.contains("older"), "{out}");
    assert!(out.contains("newer"), "{out}");
    assert!(out.contains("KiB"), "{out}");
    assert!(
        out.find("newer").unwrap() < out.find("older").unwrap(),
        "newest first: {out}"
    );
    assert!(
        out.contains(&format!("Storage root: {}", env.sessions.display())),
        "{out}"
    );
}
