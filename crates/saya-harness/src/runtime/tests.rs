use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

use saya_agent::CancellationToken;

use super::{
    NODE_IMAGE_MANIFEST_DIGEST, NODE_VERSION, NodeInstallError, Registry, RegistryError,
    install_node,
};

#[path = "cleanup_tests.rs"]
mod cleanup_tests;
#[path = "install_tests.rs"]
mod install_tests;
#[path = "npm_snapshot_tests.rs"]
mod npm_snapshot_tests;
#[path = "npm_tests.rs"]
mod npm_tests;
#[path = "own_process.rs"]
mod own_process;
#[path = "process_fixture.rs"]
mod process_fixture;
#[path = "reconcile_tests.rs"]
mod reconcile_tests;
#[path = "registry_safety_tests.rs"]
mod registry_safety_tests;

#[cfg(unix)]
#[test]
fn sticky_bit_matches_the_platform_libc_value() {
    assert_eq!(u64::from(super::STICKY_BIT), u64::from(libc::S_ISVTX));
}

#[test]
fn private_registry_never_replaces_an_existing_ready_record() {
    // Contract: a competing/previous ready record is retained, not overwritten.
    // Regression: create-or-truncate publication could silently replace a proved
    // runtime with a different one. This is the durable registry owner, not an
    // in-memory stand-in.
    let root = private_root("no-replace");
    let registry = Registry::open(&root).expect("private registry opens");
    registry.publish().expect("first publication succeeds");
    drop(registry);

    let registry = Registry::open(&root).expect("record may be reopened");
    assert!(registry.has_ready().expect("ready record is exact"));
    assert_eq!(registry.publish(), Err(RegistryError::Conflict));
    assert!(registry.has_ready().expect("original record is retained"));
    drop(registry);
    fs::remove_dir_all(root).expect("test-owned root removed");
}

#[test]
fn wrong_durable_runtime_record_is_retained_as_a_conflict() {
    // Contract: an existing record is never trusted from its filename alone.
    // Regression: a valid Node value accidentally hiding a wrong npm value.
    let root = private_root("wrong-npm");
    let runtime = root.join("managed-node-runtime");
    fs::create_dir(&runtime).expect("private runtime directory");
    #[cfg(unix)]
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).expect("private mode");
    fs::write(
        runtime.join("ready"),
        format!("image={NODE_IMAGE_MANIFEST_DIGEST}\nnode={NODE_VERSION}\nnpm=0.0.0\n"),
    )
    .expect("test record");
    #[cfg(unix)]
    fs::set_permissions(runtime.join("ready"), fs::Permissions::from_mode(0o600))
        .expect("private mode");

    let registry = Registry::open(&root).expect("registry opens without replacing record");
    assert_eq!(registry.has_ready(), Err(RegistryError::Conflict));
    drop(registry);
    fs::remove_dir_all(root).expect("test-owned root removed");
}

#[test]
fn private_registry_authfile_is_a_valid_empty_auth_document() {
    // The real Registry owner supplies an explicit credential-free authfile to
    // every engine command; a zero-byte placeholder is not an auth document.
    let root = private_root("empty-auth");
    let registry = Registry::open(&root).expect("private registry opens");
    let auth = fs::read(registry.authfile()).expect("private authfile is readable");
    let document: serde_json::Value =
        serde_json::from_slice(&auth).expect("authfile is a valid JSON document");
    let fields = document
        .as_object()
        .expect("authfile document has an object root");
    assert_eq!(fields.len(), 1, "no credential fields are permitted");
    assert!(
        fields["auths"]
            .as_object()
            .expect("auths is an object")
            .is_empty(),
        "the explicit authfile contains no credentials"
    );
    drop(registry);
    fs::remove_dir_all(root).expect("test-owned root removed");
}

#[cfg(unix)]
#[test]
fn registry_creates_a_missing_state_root_without_chmodding_its_parent() {
    // Fresh installs create only the missing state leaf with private mode and
    // leave the already-owned parent permissions untouched.
    let parent = private_root("state-parent");
    let state_root = parent.join("state");
    let parent_mode = fs::metadata(&parent).expect("parent metadata").mode() & 0o777;

    let registry = Registry::open(&state_root).expect("fresh private state root is created");

    assert_eq!(fs::metadata(&state_root).unwrap().mode() & 0o777, 0o700);
    assert_eq!(fs::metadata(&parent).unwrap().mode() & 0o777, parent_mode);
    drop(registry);
    fs::remove_dir_all(parent).expect("test-owned root removed");
}

#[cfg(unix)]
#[test]
fn registry_refuses_a_private_state_leaf_under_a_publicly_writable_parent() {
    let sandbox = private_root("unsafe-parent");
    let parent = sandbox.join("shared");
    fs::create_dir(&parent).expect("test-owned parent created");
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o777))
        .expect("test-owned parent is made public-writable");
    let state_root = parent.join("state");
    fs::create_dir(&state_root).expect("private state leaf created");
    fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700))
        .expect("state leaf is private");

    assert!(matches!(
        Registry::open(&state_root),
        Err(RegistryError::InvalidStateRoot)
    ));

    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))
        .expect("test-owned parent is private again");
    fs::remove_dir_all(sandbox).expect("test-owned root removed");
}

#[test]
fn persistent_registry_lock_is_released_after_owner_process_crashes() {
    // A process kill skips Drop; the OS lock must release while the persistent
    // lock file remains available for the next installer.
    let root = private_root("crash-lock");
    let marker = root.join("lock-held");
    let mut child = Command::new(std::env::current_exe().expect("test executable resolves"))
        .args(["--exact", "runtime::tests::hold_registry_lock_in_child"])
        .env_clear()
        .env("SAYA_TEST_RUNTIME_STATE_ROOT", &root)
        .env("SAYA_TEST_RUNTIME_LOCK_MARKER", &marker)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("isolated lock holder starts");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !marker.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    if !marker.exists() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("child did not acquire the persistent registry lock");
    }
    child.kill().expect("lock holder is terminated");
    let status = child.wait().expect("lock holder is reaped");
    assert!(!status.success(), "the lock holder was killed before Drop");

    let registry = Registry::open(&root)
        .expect("the OS releases the persistent lock after owner process exit");
    assert!(root.join("managed-node-runtime/install.lock").exists());
    drop(registry);
    fs::remove_dir_all(root).expect("test-owned root removed");
}

#[tokio::test]
async fn unknown_probe_cleanup_retains_its_container_locator() {
    let root = private_root("unknown-probe-cleanup");
    let registry = Registry::open(&root).expect("private registry opens");
    let executable = process_fixture::compile_fake_podman(registry.root());
    fs::write(
        registry.root().join("fake-plan"),
        "run_exit=1\nrm_exit=125\nexists_exit=125\n",
    )
    .expect("fake plan written");
    let podman = super::podman::Podman::from_candidate(executable, Some(registry.authfile()))
        .expect("compiled fake process passes the concrete path checks");
    let runtime_root = registry.root().to_path_buf();

    let result = podman
        .probe_node(&registry, &CancellationToken::new())
        .await;

    assert!(matches!(
        result,
        Err(super::podman::PodmanError::CleanupUnknown { .. })
    ));
    let ticket = fs::read_to_string(process_fixture::container_record_path(&runtime_root))
        .expect("fake process retained its container facts")
        .lines()
        .nth(1)
        .expect("fake process recorded the owner ticket")
        .to_owned();
    assert!(
        runtime_root.join(format!("container-{ticket}")).exists(),
        "unknown cleanup must preserve the cidfile for its own ticket"
    );
    drop(registry);
    fs::remove_dir_all(root).expect("test-owned root removed");
}

#[test]
fn hold_registry_lock_in_child() {
    let (Ok(state_root), Ok(marker)) = (
        env::var("SAYA_TEST_RUNTIME_STATE_ROOT"),
        env::var("SAYA_TEST_RUNTIME_LOCK_MARKER"),
    ) else {
        return;
    };
    let _registry = Registry::open(Path::new(&state_root)).expect("child acquires lock");
    fs::write(marker, b"held").expect("parent sees the acquired lock");
    thread::sleep(Duration::from_secs(30));
}

#[tokio::test]
async fn missing_consent_has_no_engine_or_private_state_effect() {
    // Contract: --yes is required before engine discovery or state creation.
    // A nonexistent root proves this without a Podman fixture or host command.
    let root = std::env::temp_dir().join(format!("saya-node-no-consent-{}", nonce()));
    let result = install_node(&root, None, &CancellationToken::new()).await;
    assert_eq!(result, Err(NodeInstallError::ConsentRequired));
    assert!(!root.exists(), "missing consent must not create state");
}

fn private_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-node-{label}-{}", nonce()));
    fs::create_dir(&root).expect("test root");
    #[cfg(unix)]
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("private mode");
    fs::canonicalize(root).expect("test root canonicalizes")
}

/// Unique across processes: the clock ticks in microseconds on macOS, and two
/// test processes started together can reach the same sequence number at the
/// same tick, so the process id keeps their roots apart.
fn nonce() -> u128 {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos()
        << 64;
    let sequence = (u64::from(std::process::id()) << 32) | SEQUENCE.fetch_add(1, Ordering::Relaxed);
    timestamp | u128::from(sequence)
}
