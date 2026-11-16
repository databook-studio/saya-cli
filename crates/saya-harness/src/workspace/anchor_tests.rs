#![cfg(unix)]

use std::{
    env,
    ffi::CString,
    fs,
    os::unix::ffi::OsStrExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{HarnessError, workspace::Workspace};

const CHILD_ROOT: &str = "SAYA_TEST_ANCHOR_FIFO_CHILD_ROOT";
const CHILD_TEST: &str = "workspace::anchor::tests::verified_open_refuses_fifo_swapped_after_scan";

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after the epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "saya-anchor-fifo-{}-{nonce}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create isolated FIFO test root");
        Self(path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn verified_open_refuses_fifo_swapped_after_scan() {
    if let Some(root) = env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root);
        exercise_fifo_swap(&root);
        fs::write(root.join("child-complete"), b"ok").expect("record completed child test");
        return;
    }

    let root = TempRoot::new();
    fs::write(root.0.join("victim"), b"scanned regular file").unwrap();
    let executable = env::current_exe().expect("current test executable");
    let mut child = Command::new(executable)
        .arg("--exact")
        .arg(CHILD_TEST)
        .env(CHILD_ROOT, &root.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn bounded FIFO self-test");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                assert!(status.success(), "FIFO child test failed: {status}");
                let marker = fs::read(root.0.join("child-complete"))
                    .expect("the exact child test must execute and complete");
                assert_eq!(marker, b"ok");
                return;
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let status = child.wait().expect("reap timed-out FIFO self-test");
                panic!("FIFO self-test exceeded 5 seconds and was reaped: {status}");
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("poll FIFO self-test: {error}");
            }
        }
    }
}

fn exercise_fifo_swap(root: &Path) {
    let workspace = Workspace::open(root).expect("open test workspace");
    let anchor = workspace
        .anchor("victim", false)
        .expect("scan original regular file");
    let scanned = anchor.stat().expect("scanned file exists");
    assert!(scanned.is_file());
    let identity = scanned.identity();

    let victim = root.join("victim");
    fs::remove_file(&victim).expect("remove scanned regular file");
    let victim_c = CString::new(victim.as_os_str().as_bytes()).expect("FIFO path has no NUL");
    let result = unsafe { libc::mkfifo(victim_c.as_ptr(), 0o600) };
    assert_eq!(
        result,
        0,
        "create FIFO: {}",
        std::io::Error::last_os_error()
    );

    assert!(matches!(
        anchor.open_verified(identity, "victim"),
        Err(HarnessError::NotRegularFile { .. })
    ));
}
