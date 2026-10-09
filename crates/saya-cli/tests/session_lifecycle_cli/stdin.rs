use super::support::{saya, test_env};
use std::{
    io::Read,
    process::Stdio,
    time::{Duration, Instant},
};

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
