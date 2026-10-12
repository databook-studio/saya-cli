use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const HELPER: &str = "saya-c02-helper";
pub const CREDENTIAL_ENV: &str = "SAYA_C02_TOKEN";
/// A planted fake value, not a real secret: it exists so the redaction gate
/// has something to catch, in the M5-7 battery's shape.
pub const CREDENTIAL_VALUE: &str = "planted-c02-7Qk2mVn9xR4";
const CREDENTIAL_REF: &str = "c02-token-value";

/// Creates `path` and forces it private: the store refuses an existing
/// directory whose group or other access is non-zero, and a usual 022 umask
/// clears nothing from a fresh `create_dir_all`, so both are set explicitly
/// to make each refusal test fail for its own reason.
pub fn create_private_dir(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

/// The directory names directly under `path`, sorted: the refusal tests
/// compare snapshots of a store root across a refused start.
pub fn sorted_entries(path: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The flood pattern the helper writes: byte `i` of a stream is `a`-plus-(i
/// mod 26) on stdout, `A`-plus-(i mod 26) on stderr, so the stored head can
/// be compared byte-for-byte against what the child wrote first.
pub fn expected_flood_prefix(len: usize, upper: bool) -> Vec<u8> {
    let base = if upper { b'A' } else { b'a' };
    (0..len).map(|i| base + (i % 26) as u8).collect()
}

/// No prefix or suffix fragment of eight or more bytes of the planted value,
/// and not the whole value, may appear in a stored stream snapshot.
pub fn assert_no_credential_fragment(stored: &str, context: &str) {
    assert!(
        !stored.contains(CREDENTIAL_VALUE),
        "{context} holds the whole credential value: {stored:?}"
    );
    for len in 8..CREDENTIAL_VALUE.len() {
        let prefix = &CREDENTIAL_VALUE[..len];
        let suffix = &CREDENTIAL_VALUE[CREDENTIAL_VALUE.len() - len..];
        assert!(
            !stored.contains(prefix) && !stored.contains(suffix),
            "{context} holds the {len}-byte fragment {prefix:?} or {suffix:?}: {stored:?}"
        );
    }
}

/// Re-executes this test binary for the exact test `child_test` with
/// `child_env` set to `done`, and waits, bounded, for the child to exit
/// successfully and write the completion marker: process-wide test state
/// (rlimits, signal dispositions) stays confined to the child process.
pub fn run_test_scenario_in_child(child_env: &str, child_test: &str, done: &Path) {
    let mut child = Command::new(env::current_exe().expect("the test binary path resolves"))
        .arg("--exact")
        .arg(child_test)
        .env(child_env, done)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the isolated scenario child spawns");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                assert!(
                    status.success(),
                    "the isolated scenario child failed: {status}"
                );
                assert_eq!(
                    fs::read(done).expect("the child scenario ran to completion"),
                    b"ok",
                    "the exact child test must execute and complete"
                );
                return;
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = (child.kill(), child.wait());
                panic!("the scenario child exceeded 60 s and was reaped");
            }
            Err(error) => {
                let _ = (child.kill(), child.wait());
                panic!("polling the scenario child failed: {error}");
            }
        }
    }
}

#[path = "c02_fixture.rs"]
mod fixture_impl;
pub use fixture_impl::{Fixture, fixture};
