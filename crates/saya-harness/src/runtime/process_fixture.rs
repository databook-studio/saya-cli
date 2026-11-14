//! Compiles the concrete Podman process fixture used by runtime owner tests.
//!
//! The fixture is compiled and run once per test process; each test gets a hard
//! link to it. macOS vets an executable the first time it runs, one at a time
//! for the whole machine, and the vetting of a binary compiled by every test
//! queued each test's first run behind the others' inside the owners' 5 s and
//! 20 s bounds. A hard link shares the vetted inode, so a test's first run of
//! its link is as quick as any later one.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

/// What the fixture exits with when it is run without a registry authfile.
const NO_AUTHFILE_EXIT: i32 = 97;

/// Names the vetted fixture a parent test process hands to a process it
/// re-executes, so the child links to it instead of compiling its own.
pub(super) const SHARED_ENV: &str = "SAYA_TEST_FAKE_PODMAN";

/// The fixture shared by every test in this process, built and run once.
pub(super) fn shared_fixture() -> &'static Path {
    static SHARED: OnceLock<PathBuf> = OnceLock::new();
    SHARED.get_or_init(|| {
        std::env::var_os(SHARED_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(build_shared_fixture)
    })
}

pub(super) fn compile_fake_podman(root: &Path) -> PathBuf {
    let shared = shared_fixture();
    let executable = root.join("fake-podman");
    if fs::hard_link(shared, &executable).is_err() {
        fs::copy(shared, &executable).expect("fake Podman is copied beside the test");
        #[cfg(unix)]
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .expect("fake Podman is executable by the owner");
    }
    executable
}

fn build_shared_fixture() -> PathBuf {
    let directory = std::env::temp_dir().join(format!("saya-fake-podman-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("fake Podman directory is created");
    let source = directory.join("fake-podman.rs");
    let executable = directory.join("fake-podman");
    fs::write(&source, include_str!("fake_podman.rs")).expect("fake Podman source is written");
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let status = Command::new(rustc)
        .arg("--edition=2024")
        .arg("-o")
        .arg(&executable)
        .arg(&source)
        .status()
        .expect("fake Podman process compiles");
    assert!(status.success(), "fake Podman compile succeeds: {status}");
    #[cfg(unix)]
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
        .expect("fake Podman is executable by the owner");
    run_once(&executable);
    executable
}

/// Runs the fixture to completion so the machine has vetted it before any test
/// is timed. A run the system kills proves nothing about the fixture, so it is
/// repeated a few times before it fails the build.
fn run_once(executable: &Path) {
    let mut last = None;
    for _ in 0..3 {
        let status = Command::new(executable)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("fake Podman starts");
        if status.code() == Some(NO_AUTHFILE_EXIT) {
            return;
        }
        last = Some(status);
    }
    panic!("fake Podman did not complete its first run: {last:?}");
}

pub(super) fn container_record_path(root: &Path) -> PathBuf {
    root.join("fake-owned-container-state")
}

pub(super) fn write_container_record(root: &Path, ticket: &str, name: &str) {
    let id = "a".repeat(64);
    fs::write(
        container_record_path(root),
        format!("{id}\n{ticket}\n{name}\n"),
    )
    .expect("fake Podman container record is written");
}
