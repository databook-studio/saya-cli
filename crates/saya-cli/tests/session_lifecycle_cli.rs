//! `saya sessions list|delete` through the real binary.
//!
//! Saved-session management must be explicit and never automatic: listing
//! mutates nothing, delete removes exactly the named saved session (its
//! record, state directory, and transient copies) and says plainly what it
//! did not delete, a live session is refused by pid, and no path ever
//! prompts or reads stdin. Every refusal exits 2, the documented usage
//! class.

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{Duration, Instant, SystemTime},
};

struct TestEnv {
    root: PathBuf,
    sessions: PathBuf,
}

fn test_env(label: &str) -> TestEnv {
    let root = std::env::temp_dir().join(format!(
        "saya-cli-session-lifecycle-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let sessions = root.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    TestEnv { root, sessions }
}

fn saya(env: &TestEnv, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args(args)
        .env("SAYA_SESSION_DIR", &env.sessions)
        .env("SAYA_STATE_DB", env.root.join("state.sqlite3"))
        .env("SAYA_CONFIG_HOME", env.root.join("user-config"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn run(env: &TestEnv, args: &[&str]) -> Output {
    saya(env, args).output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn entries(path: &Path) -> Vec<String> {
    let mut names = std::fs::read_dir(path)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// Pins the file's modification time to `ago` before now, so listing order
/// is deterministic regardless of write timing.
fn age(path: &Path, ago: Duration) {
    let mtime = SystemTime::now().checked_sub(ago).unwrap();
    let file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    file.set_modified(mtime).unwrap();
}

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

/// The command never reads stdin: run with an open, empty pipe (a read
/// would block forever) and `--non-interactive`, it completes on its own.
#[test]
fn the_command_never_reads_stdin() {
    let env = test_env("no-stdin");
    let mut child = saya(&env, &["--non-interactive", "sessions", "list"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    // Held open and never written: any stdin read would block forever.
    let _held_open = child.stdin.take();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "the command blocked waiting on stdin"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(status.code(), Some(0), "list completed without stdin");
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    assert!(stdout.contains("No saved sessions."), "{stdout}");
    let _ = std::fs::remove_dir_all(&env.root);
}
