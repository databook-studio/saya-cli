//! A credentials-parameterized fixture for the C02b cursor slice: the
//! shared `c02_support` fixture pins one 22-byte credential value, while
//! the refused-redactor test needs a declared value below the redactor's
//! minimum-secret length, so this file compiles its own echo helper and
//! builds `RunProgram` with that value.

use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use saya_harness::{
    runner::sandbox::{RunSandbox, RunnerSpawn, SandboxProvision},
    runner::{Credential, RunProgram, StaticCredentialSource},
};
use saya_types::{RunnerScope, SecretRef};

pub const HELPER: &str = "saya-c02-cursor-helper";
pub const CREDENTIAL_ENV: &str = "SAYA_C02_TOKEN";
/// Three bytes: below the redactor's minimum accepted secret length, so the
/// gate refuses the credential set at construction and seals the stream.
pub const SHORT_CREDENTIAL_VALUE: &str = "abc";
const CREDENTIAL_REF: &str = "c02-cursor-token-value";

const HELPER_SOURCE: &str = r#"
use std::{env, io::{self, Write}, process};

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("echo-credential") => {
            let name = args.next().unwrap();
            let value = env::var(&name).unwrap_or_default();
            println!("{name}={value}");
            io::stdout().flush().unwrap();
        }
        _ => process::exit(112),
    }
}
"#;

static NEXT_MARKER: AtomicU64 = AtomicU64::new(0);

pub struct CursorFixture {
    workspace: PathBuf,
    spawn: RunnerSpawn,
}

impl CursorFixture {
    /// One `RunProgram` allowlisting the cursor helper and declaring the
    /// three-byte credential: only its resolved value reaches a child's
    /// environment, and it is exactly what the redaction gate must refuse.
    pub fn run_program(&self) -> RunProgram {
        let scope = RunnerScope::new(vec![HELPER.to_owned()]).expect("helper name is valid");
        let credential = Credential::new(
            "c02-cursor-token",
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
                SHORT_CREDENTIAL_VALUE.to_owned(),
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

pub fn fixture() -> Option<&'static CursorFixture> {
    static FIXTURE: OnceLock<Option<CursorFixture>> = OnceLock::new();
    FIXTURE.get_or_init(build_fixture).as_ref()
}

fn build_fixture() -> Option<CursorFixture> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after epoch")
        .as_nanos();
    let root = env::temp_dir().join(format!("saya-c02-cursor-{}-{stamp}", std::process::id()));
    let workspace = root.join("workspace");
    let programs = root.join("programs");
    fs::create_dir_all(&workspace).expect("workspace is created");
    fs::create_dir_all(&programs).expect("program directory is created");
    fs::set_permissions(&workspace, fs::Permissions::from_mode(0o700))
        .expect("workspace is private");
    fs::set_permissions(&programs, fs::Permissions::from_mode(0o700))
        .expect("program directory is private");

    let source = root.join("c02-cursor-helper.rs");
    let binary = programs.join(HELPER);
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
                "C02 cursor skipped: host sandbox is not proven\n{}",
                report.render()
            );
            return None;
        }
    };
    Some(CursorFixture { workspace, spawn })
}
