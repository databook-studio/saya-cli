use super::support::*;
use std::{process::Command, time::SystemTime};

/// Deleting one saved session removes its record and state directory and
/// nothing else, exits 0, and prints the retained-data sentence.
#[test]
fn delete_removes_exactly_one_session_and_says_what_it_kept() {
    let env = test_env("delete");
    std::fs::write(env.sessions.join("a.json"), "{}").unwrap();
    std::fs::create_dir_all(env.sessions.join("a")).unwrap();
    std::fs::write(env.sessions.join("a").join("state.txt"), "x").unwrap();
    std::fs::write(env.sessions.join("other.json"), "{}").unwrap();
    std::fs::write(env.root.join("state.sqlite3"), "sentinel").unwrap();
    let output = run(&env, &["sessions", "delete", "a"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        stdout(&output),
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains(
            "Project preferences and typed database knowledge are stored \
             separately and were not deleted."
        ),
        "{out}"
    );
    assert!(!env.sessions.join("a.json").exists(), "record removed");
    assert!(!env.sessions.join("a").exists(), "state dir removed");
    assert!(
        env.sessions.join("other.json").exists(),
        "other session kept"
    );
    assert_eq!(
        std::fs::read_to_string(env.root.join("state.sqlite3")).unwrap(),
        "sentinel",
        "independent data kept"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// Deleting an id with no saved session says so (exit 2) and creates
/// nothing: no transient state dir, no tombstone.
#[test]
fn deleting_an_absent_session_is_a_refusal_that_creates_nothing() {
    let env = test_env("absent");
    let output = run(&env, &["sessions", "delete", "nope"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout: {}\nstderr: {}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("no saved session"),
        "{}",
        stderr(&output)
    );
    assert!(
        entries(&env.sessions).is_empty(),
        "nothing created: {:?}",
        entries(&env.sessions)
    );
    assert!(
        !env.root.join("lifecycle").exists(),
        "no tombstone was written"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// An invalid id (`../x`) is refused with a usage-style message and no
/// filesystem effect anywhere: nothing outside the sessions root either.
#[test]
fn an_invalid_id_is_refused_with_no_filesystem_effect() {
    let env = test_env("invalid");
    let output = run(&env, &["sessions", "delete", "../x"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout: {}\nstderr: {}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("not a valid session id"),
        "{}",
        stderr(&output)
    );
    assert!(
        entries(&env.sessions).is_empty(),
        "nothing created inside: {:?}",
        entries(&env.sessions)
    );
    assert!(!env.root.join("x.json").exists(), "nothing outside");
    assert!(
        !env.root.join("lifecycle").exists(),
        "no tombstone was written"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// A live session (another process holds its lock) is refused, the message
/// names the holder's pid, and all data is left intact.
#[cfg(unix)]
#[test]
fn a_live_session_is_refused_with_its_holder_pid_and_left_intact() {
    let env = test_env("live");
    std::fs::write(env.sessions.join("a.json"), "{}").unwrap();
    std::fs::create_dir_all(env.sessions.join("a")).unwrap();
    let holder = Command::new("sleep").arg("30").spawn().unwrap();
    struct KillGuard(std::process::Child);
    impl Drop for KillGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let pid = holder.id();
    let _guard = KillGuard(holder);
    std::fs::write(env.sessions.join("a").join("lock"), format!("{pid}\n")).unwrap();
    let output = run(&env, &["sessions", "delete", "a"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout: {}\nstderr: {}",
        stdout(&output),
        stderr(&output)
    );
    let err = stderr(&output);
    assert!(err.contains("pid"), "{err}");
    assert!(err.contains(&pid.to_string()), "{err}");
    assert!(env.sessions.join("a.json").exists(), "data intact");
    assert!(
        !env.root.join("lifecycle").exists(),
        "no tombstone while live"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// A tombstone-only leftover (planted with the store's own record shape:
/// `{"version":1,"id":..,"started_unix_ms":..}`, the shape tombstone.rs
/// writes) shows as a pending deletion and is finished by the same delete
/// command: exit 0 and the tombstone file is gone.
#[test]
fn deleting_a_tombstone_only_leftover_clears_the_pending_deletion() {
    let env = test_env("ghost");
    let lifecycle = env.root.join("lifecycle");
    std::fs::create_dir_all(&lifecycle).unwrap();
    let started_unix_ms = std::time::SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    std::fs::write(
        lifecycle.join("session-ghost.tombstone"),
        format!(
            "{{\n  \"version\": 1,\n  \"id\": \"ghost\",\n  \"started_unix_ms\": {started_unix_ms}\n}}"
        ),
    )
    .unwrap();
    let output = run(&env, &["sessions", "delete", "ghost"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        !lifecycle.join("session-ghost.tombstone").exists(),
        "the tombstone file is gone"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
