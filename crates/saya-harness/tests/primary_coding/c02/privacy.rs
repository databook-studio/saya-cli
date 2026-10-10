//! Public-boundary privacy tests of the command output store (C02c1): the
//! store directory and its stream files carry private modes, a symlink
//! pre-placed at a stream path is refused on read, and a credential set
//! the redactor refuses stores nothing.

use std::{
    env, fs,
    num::NonZeroU64,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    process::Command,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use saya_harness::coding::commands::{
    CommandOutputCursor, CommandOutputError, CommandOutputStatus, CommandOwner, CommandSession,
    MAX_OUTPUT_READ_BYTES, OutputStream,
};
use saya_harness::{
    runner::sandbox::{RunSandbox, RunnerSpawn, SandboxProvision},
    runner::{Credential, RunProgram, StaticCredentialSource},
};
use saya_types::{RunnerScope, SecretRef};

pub const PRIVACY_HELPER: &str = "saya-c02-privacy-helper";
pub const CREDENTIAL_ENV: &str = "SAYA_C02_PRIVACY_TOKEN";
/// The accepted declared value: long enough for the redactor's gate and
/// free of the redaction marker, so an accepted run stores real output.
pub const ACCEPTED_CREDENTIAL_VALUE: &str = "c02c1-privacy-token";
/// Declared above the redactor's minimum length but carrying the literal
/// `[redacted]` marker, so the gate refuses the credential set at
/// construction and seals the stream.
pub const REFUSED_CREDENTIAL_VALUE: &str = "c02c1-[redacted]-token";
const CREDENTIAL_REF: &str = "c02-privacy-token-value";

const HELPER_SOURCE: &str = r#"
fn main() {
    println!("c02c1-privacy-line");
}
"#;

static NEXT_MARKER: AtomicU64 = AtomicU64::new(0);

pub struct PrivacyFixture {
    workspace: PathBuf,
    spawn: RunnerSpawn,
}

impl PrivacyFixture {
    /// One `RunProgram` allowlisting the privacy helper and declaring the
    /// accepted credential, so the run stores real output.
    pub fn run_program(&self) -> RunProgram {
        self.program_with_value(ACCEPTED_CREDENTIAL_VALUE)
    }

    /// One `RunProgram` whose declared value carries the literal
    /// `[redacted]` marker: exactly what the redaction gate must refuse.
    pub fn refused_program(&self) -> RunProgram {
        self.program_with_value(REFUSED_CREDENTIAL_VALUE)
    }

    fn program_with_value(&self, value: &str) -> RunProgram {
        let scope =
            RunnerScope::new(vec![PRIVACY_HELPER.to_owned()]).expect("helper name is valid");
        let credential = Credential::new(
            "c02-privacy-token",
            CREDENTIAL_ENV,
            SecretRef::Env {
                env: CREDENTIAL_REF.to_owned(),
            },
        )
        .expect("the fixture credential is well formed");
        RunProgram::new(
            self.spawn.clone(),
            scope,
            Duration::from_secs(40),
            Arc::new(StaticCredentialSource::new([(
                CREDENTIAL_REF.to_owned(),
                value.to_owned(),
            )])),
        )
        .with_credentials(vec![credential])
    }

    /// A unique private path under the fixture workspace, for per-test
    /// store roots.
    pub fn marker(&self, label: &str) -> PathBuf {
        self.workspace.join(format!(
            "{label}-{}",
            NEXT_MARKER.fetch_add(1, Ordering::Relaxed)
        ))
    }
}

pub fn fixture() -> Option<&'static PrivacyFixture> {
    static FIXTURE: OnceLock<Option<PrivacyFixture>> = OnceLock::new();
    FIXTURE.get_or_init(build_privacy_fixture).as_ref()
}

fn build_privacy_fixture() -> Option<PrivacyFixture> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after epoch")
        .as_nanos();
    let root = env::temp_dir().join(format!("saya-c02-privacy-{}-{stamp}", std::process::id()));
    let workspace = root.join("workspace");
    let programs = root.join("programs");
    fs::create_dir_all(&workspace).expect("workspace is created");
    fs::create_dir_all(&programs).expect("program directory is created");
    fs::set_permissions(&workspace, fs::Permissions::from_mode(0o700))
        .expect("workspace is private");
    fs::set_permissions(&programs, fs::Permissions::from_mode(0o700))
        .expect("program directory is private");

    let source = root.join("c02-privacy-helper.rs");
    let binary = programs.join(PRIVACY_HELPER);
    fs::write(&source, HELPER_SOURCE).expect("native test helper source writes");
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let status = Command::new(rustc)
        .arg("--edition=2021")
        .arg("-o")
        .arg(&binary)
        .arg(&source)
        .status()
        .expect("the active Rust toolchain compiles the native test helper");
    assert!(status.success(), "native test helper compiles: {status}");

    let policy = RunSandbox::new([workspace.clone()], Vec::new())
        .expect("workspace is a valid sandbox root");
    let provision = policy
        .prepare(&programs)
        .expect("host sandbox probe completes");
    let spawn = match provision {
        SandboxProvision::Proven { spawn, .. } => spawn,
        SandboxProvision::Refused(report) => {
            eprintln!(
                "C02c1 privacy skipped: host sandbox is not proven\n{}",
                report.render()
            );
            return None;
        }
    };
    Some(PrivacyFixture { workspace, spawn })
}

/// Polls `output_status` before any join, per the call-before-join
/// contract: `Some` only once both sinks finished, so every post-finish
/// check below observes the store's final outcome.
async fn wait_for_status(session: &CommandSession) -> CommandOutputStatus {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(ready) = session.output_status() {
            return ready;
        }
        assert!(Instant::now() < deadline, "status within 60 s");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Joins a finished session and asserts the helper exited cleanly.
async fn join_exits_cleanly(session: CommandSession) {
    let outcome = tokio::time::timeout(Duration::from_secs(30), session.join())
        .await
        .expect("the run settles within 30 s")
        .unwrap();
    assert_eq!(outcome.exit_code, Some(0), "the helper exits cleanly");
}

/// (a) After a finished run with output, the store directory and both
/// stream files carry private modes: the store dir is 0o700 and each
/// stream file 0o600, read through symlink metadata.
#[tokio::test]
async fn store_files_are_private() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.run_program(),
            PRIVACY_HELPER.to_owned(),
            Vec::new(),
            NonZeroU64::new(20).unwrap(),
            fixture.marker("privacy-perms"),
        )
        .unwrap();
    wait_for_status(&session).await;
    let store_dir = session
        .output_dir()
        .expect("session carries an output store");
    let dir_mode = fs::symlink_metadata(&store_dir)
        .expect("the store dir exists")
        .permissions()
        .mode();
    assert_eq!(dir_mode & 0o777, 0o700, "the store dir is private");
    let stdout = store_dir.join(OutputStream::Stdout.file_name());
    let stdout_mode = fs::symlink_metadata(&stdout)
        .expect("the stdout stream file exists after a run with output")
        .permissions()
        .mode();
    assert_eq!(
        stdout_mode & 0o777,
        0o600,
        "the stdout stream file is private"
    );
    let stderr = store_dir.join(OutputStream::Stderr.file_name());
    let stderr_mode = fs::symlink_metadata(&stderr)
        .expect("the stderr stream file exists after a run with output")
        .permissions()
        .mode();
    assert_eq!(
        stderr_mode & 0o777,
        0o600,
        "the stderr stream file is private"
    );
    join_exits_cleanly(session).await;
}

/// (b) A symlink pre-placed at the stdout stream path after a finished run
/// is refused on read as a typed store error: the read never serves the
/// link target's text.
#[tokio::test]
async fn pre_placed_symlink_at_stream_path_is_refused_on_read() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.run_program(),
            PRIVACY_HELPER.to_owned(),
            Vec::new(),
            NonZeroU64::new(20).unwrap(),
            fixture.marker("privacy-symlink"),
        )
        .unwrap();
    wait_for_status(&session).await;
    let stdout = session
        .output_dir()
        .expect("session carries an output store")
        .join(OutputStream::Stdout.file_name());
    let decoy_dir = fixture.marker("privacy-decoy");
    fs::create_dir_all(&decoy_dir).expect("the decoy dir is created");
    let decoy = decoy_dir.join(OutputStream::Stdout.file_name());
    fs::copy(&stdout, &decoy).expect("the stream copy is made");
    let copied = fs::read_to_string(&decoy).expect("the decoy copy is readable");
    assert!(
        copied.contains("c02c1-privacy-line"),
        "the decoy holds the stream's text: {copied:?}"
    );
    fs::remove_file(&stdout).expect("the stream file is removed");
    symlink(&decoy, &stdout).expect("the symlink is placed");
    let result = session.read_output(
        CommandOutputCursor::start(OutputStream::Stdout),
        MAX_OUTPUT_READ_BYTES,
    );
    assert!(
        matches!(result, Err(CommandOutputError::Store { .. })),
        "a symlink at the stream path is refused as a store error, never the target's text"
    );
    join_exits_cleanly(session).await;
}

/// (c) A credential value carrying the literal `[redacted]` marker makes
/// the redactor refuse the secret set at construction: the stream seals
/// with no stored bytes — the stdout file stays empty — and the start
/// cursor reads empty at end-of-stream.
#[tokio::test]
async fn refused_secret_set_stores_nothing() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.refused_program(),
            PRIVACY_HELPER.to_owned(),
            Vec::new(),
            NonZeroU64::new(20).unwrap(),
            fixture.marker("privacy-refused"),
        )
        .unwrap();
    let status = wait_for_status(&session).await;
    assert!(
        status.capped && status.error.is_none(),
        "a refused redactor seals its stream without a store error: {status:?}"
    );
    let stdout = session
        .output_dir()
        .expect("session carries an output store")
        .join(OutputStream::Stdout.file_name());
    let len = fs::symlink_metadata(&stdout)
        .expect("the stdout stream file exists")
        .len();
    assert_eq!(len, 0, "the refused run stores no stdout bytes");
    let read = session
        .read_output(
            CommandOutputCursor::start(OutputStream::Stdout),
            MAX_OUTPUT_READ_BYTES,
        )
        .expect("reading a sealed stream at the start cursor succeeds");
    assert!(
        read.text().is_empty() && read.end_of_stream(),
        "the sealed stream reads empty at end-of-stream: {:?}",
        read.text()
    );
    join_exits_cleanly(session).await;
}
