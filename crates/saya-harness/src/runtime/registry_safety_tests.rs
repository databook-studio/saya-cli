#![cfg(unix)]

use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::{
    ffi::CString,
    fs,
    os::unix::ffi::OsStrExt as _,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use super::private_root;

#[test]
fn registry_refuses_a_private_leaf_with_a_writable_intermediate_ancestor() {
    let sandbox = private_root("unsafe-intermediate");
    let ancestor = sandbox.join("shared");
    let parent = ancestor.join("private");
    let state_root = parent.join("state");
    fs::create_dir(&ancestor).expect("test-owned ancestor created");
    fs::set_permissions(&ancestor, fs::Permissions::from_mode(0o777))
        .expect("test-owned ancestor is public-writable");
    fs::create_dir(&parent).expect("private immediate parent created");
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))
        .expect("immediate parent is private");
    fs::create_dir(&state_root).expect("private leaf created");
    fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700))
        .expect("state leaf is private");

    assert!(matches!(
        super::super::Registry::open(&state_root),
        Err(super::super::RegistryError::InvalidStateRoot)
    ));

    fs::remove_dir_all(sandbox).expect("test-owned tree removed");
}

#[cfg(unix)]
#[test]
fn registry_creates_missing_private_state_ancestors_beneath_existing_root() {
    let trusted = private_root("missing-state-hierarchy");
    let state_root = trusted.join(".local/share/saya");
    let trusted_mode = fs::metadata(&trusted)
        .expect("trusted root metadata")
        .mode()
        & 0o777;

    let registry = super::super::Registry::open(&state_root)
        .expect("fresh data/state suffix is created beneath trusted root");

    for directory in [
        trusted.join(".local"),
        trusted.join(".local/share"),
        state_root.clone(),
    ] {
        assert_eq!(fs::metadata(directory).unwrap().mode() & 0o777, 0o700);
    }
    assert_eq!(fs::metadata(&trusted).unwrap().mode() & 0o777, trusted_mode);
    drop(registry);
    fs::remove_dir_all(trusted).expect("test-owned root removed");
}

/// A child that has not reached its test body within this bound never ran: a
/// loaded host can starve a freshly started process of the CPU for seconds, and
/// that says nothing about whether the registry blocks.
const FIFO_CHILD_START_BOUND: Duration = Duration::from_secs(30);

/// Once the child is in its test body, opening the registry takes milliseconds.
/// An open that blocks on the FIFO never returns, so any finite bound tells the
/// two apart; this one is far above anything a loaded host adds.
const FIFO_OPEN_BOUND: Duration = Duration::from_secs(10);

enum Marker {
    Present,
    Exited(ExitStatus),
    TimedOut,
}

fn wait_for_marker(child: &mut Child, marker: &Path, bound: Duration) -> Marker {
    let deadline = Instant::now() + bound;
    loop {
        if marker.exists() {
            return Marker::Present;
        }
        if let Some(status) = child.try_wait().expect("registry child is polled") {
            // The child may have written the marker just before it exited.
            return if marker.exists() {
                Marker::Present
            } else {
                Marker::Exited(status)
            };
        }
        if Instant::now() >= deadline {
            return Marker::TimedOut;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn fail_fifo_child(child: &mut Child, stderr: &Path, what: &str) -> ! {
    let _ = child.kill();
    let _ = child.wait();
    let stderr = fs::read_to_string(stderr).unwrap_or_default();
    panic!("{what}\nchild stderr:\n{stderr}");
}

#[test]
fn registry_does_not_block_while_checking_a_fifo_ready_path() {
    let root = private_root("fifo-ready");
    let runtime = root.join("managed-node-runtime");
    fs::create_dir(&runtime).expect("runtime directory created");
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))
        .expect("runtime directory is private");
    let ready = CString::new(runtime.join("ready").as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(ready.as_ptr(), 0o600) }, 0);
    let started = root.join("fifo-started");
    let marker = root.join("fifo-result");
    let stderr = root.join("fifo-child-stderr");
    let mut child = Command::new(std::env::current_exe().expect("test executable resolves"))
        .args([
            "--exact",
            "runtime::tests::registry_safety_tests::open_fifo_registry_in_child",
        ])
        .env_clear()
        .env("SAYA_TEST_FIFO_STATE_ROOT", &root)
        .env("SAYA_TEST_FIFO_STARTED", &started)
        .env("SAYA_TEST_FIFO_RESULT", &marker)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(fs::File::create(&stderr).expect("stderr file")))
        .spawn()
        .expect("isolated registry child starts");
    match wait_for_marker(&mut child, &started, FIFO_CHILD_START_BOUND) {
        Marker::Present => {}
        Marker::Exited(status) => fail_fifo_child(
            &mut child,
            &stderr,
            &format!("registry child exited early: {status}"),
        ),
        Marker::TimedOut => fail_fifo_child(
            &mut child,
            &stderr,
            "registry child never reached its test body",
        ),
    }
    match wait_for_marker(&mut child, &marker, FIFO_OPEN_BOUND) {
        Marker::Present => {}
        Marker::Exited(status) => fail_fifo_child(
            &mut child,
            &stderr,
            &format!("registry child exited without a result: {status}"),
        ),
        Marker::TimedOut => fail_fifo_child(
            &mut child,
            &stderr,
            "registry open blocked while checking the FIFO record",
        ),
    }
    let status = child.wait().expect("registry child is reaped");
    assert!(status.success(), "registry child returned a bounded result");
    assert_eq!(fs::read(&marker).unwrap(), b"conflict");
    fs::remove_dir_all(root).expect("test-owned root removed");
}

#[test]
fn open_fifo_registry_in_child() {
    let (Ok(state_root), Ok(started), Ok(result_path)) = (
        std::env::var("SAYA_TEST_FIFO_STATE_ROOT"),
        std::env::var("SAYA_TEST_FIFO_STARTED"),
        std::env::var("SAYA_TEST_FIFO_RESULT"),
    ) else {
        return;
    };
    fs::write(started, b"started").expect("the parent sees the child reach its test body");
    let registry = super::super::Registry::open(std::path::Path::new(&state_root))
        .expect("private registry opens before ready inspection");
    let result = registry.has_ready();
    let marker = if matches!(result, Err(super::super::RegistryError::Conflict)) {
        b"conflict".as_slice()
    } else {
        b"unexpected".as_slice()
    };
    fs::write(result_path, marker).expect("bounded result is observable");
}
