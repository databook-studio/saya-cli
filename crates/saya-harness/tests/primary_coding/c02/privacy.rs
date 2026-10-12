//! Public-boundary privacy tests of the command output store (C02c1): the
//! store directory and its stream files carry private modes, a symlink
//! pre-placed at a stream path is refused on read, and a credential set
//! the redactor refuses stores nothing.

use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use saya_harness::coding::commands::{CommandOutputStatus, CommandSession};
use saya_harness::{
    runner::sandbox::{RunSandbox, RunnerSpawn, SandboxProvision},
    runner::{Credential, RunProgram, StaticCredentialSource},
};
use saya_types::{RunnerScope, SecretRef};

#[path = "privacy/permissions.rs"]
mod permissions;
#[path = "privacy/refused_secret.rs"]
mod refused_secret;
#[path = "privacy/symlink_refusal.rs"]
mod symlink_refusal;

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
