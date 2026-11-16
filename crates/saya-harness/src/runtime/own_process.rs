//! Runs a runtime test in a process of its own.
//!
//! A child that is being spawned holds a copy of every descriptor its process
//! has open, close-on-exec or not, until its exec completes, so a lock whose
//! owner has already closed it stays held for the length of that spawn: half a
//! millisecond on an idle host, longer under load. A test that drops its
//! registry and opens it again finds the lock busy whenever a sibling test is
//! spawning at that moment. Nothing else runs in the process a test gets here,
//! so nothing else spawns while its lock is open.

use std::{
    fs,
    future::Future,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use super::{private_root, process_fixture};

/// Set on the process the test re-executes. Its value is the file that process
/// writes once its body has completed, and its presence tells the test that it
/// is that process.
const CHILD_MARKER: &str = "SAYA_TEST_OWN_PROCESS_MARKER";

/// Only a hang takes this long: the isolated bodies run for well under a
/// second on an idle host, and a re-executed test binary starts in a few
/// seconds at worst on a loaded one.
const HANG_BOUND: Duration = Duration::from_secs(120);

/// Runs `body` in a re-executed copy of this test binary that runs only the
/// test `module::name`, and passes the test when that process does. `module`
/// is the caller's `module_path!()`. A child that does not run its body to
/// completion fails the test, so a wrong `name` cannot pass by running nothing.
/// `fake_podman` hands the child this process's vetted fake Podman.
pub(super) fn run<F: Future<Output = ()>>(module: &str, name: &str, fake_podman: bool, body: F) {
    if let Some(marker) = std::env::var_os(CHILD_MARKER) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("the isolated test's runtime starts")
            .block_on(body);
        fs::write(marker, b"completed").expect("record the completed child run");
        return;
    }
    let path = module.split_once("::").map_or(module, |(_, path)| path);
    let test = format!("{path}::{name}");
    let root = private_root("own-process");
    let marker = root.join("completed");
    let stderr = root.join("stderr");
    let mut command = Command::new(std::env::current_exe().expect("test executable resolves"));
    command
        .args(["--exact", &test, "--nocapture"])
        .env(CHILD_MARKER, &marker)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(fs::File::create(&stderr).expect("stderr file")));
    if fake_podman {
        command.env(
            process_fixture::SHARED_ENV,
            process_fixture::shared_fixture(),
        );
    }
    let mut child = command.spawn().expect("isolated test starts");
    let deadline = Instant::now() + HANG_BOUND;
    let status = loop {
        match child.try_wait().expect("isolated test is polled") {
            Some(status) => break status,
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("isolated test {test} exceeded {HANG_BOUND:?} and was reaped");
            }
        }
    };
    let stderr = fs::read_to_string(&stderr).unwrap_or_default();
    assert!(
        status.success(),
        "isolated test {test} failed: {status}\n{stderr}"
    );
    assert_eq!(
        fs::read(&marker).expect("the exact child test must run to completion"),
        b"completed",
        "isolated test {test} did not run its body"
    );
    fs::remove_dir_all(root).expect("test-owned root removed");
}
