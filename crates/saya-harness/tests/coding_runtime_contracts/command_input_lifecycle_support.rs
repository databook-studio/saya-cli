use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use saya_harness::{
    runner::sandbox::{RunSandbox, RunnerSpawn, SandboxProvision},
    runner::{RunProgram, StaticCredentialSource},
};
use saya_types::RunnerScope;
use std::sync::Arc;

pub const HELPER: &str = "saya-c01b-helper";
const HELPER_SOURCE: &str = r#"
use std::{env, fs, io::{self, Read}, process, thread, time::Duration};

fn record_pid(path: &str) {
    fs::write(path, process::id().to_string()).unwrap();
}

fn summary(bytes: &[u8]) {
    let sum: u64 = bytes.iter().map(|byte| u64::from(*byte)).sum();
    println!("{}:{sum}", bytes.len());
}

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("echo") => {
            let marker = args.next().unwrap();
            record_pid(&marker);
            thread::sleep(Duration::from_millis(100));
            let mut bytes = Vec::new();
            io::stdin().read_to_end(&mut bytes).unwrap();
            summary(&bytes);
        }
        Some("hold") => {
            let marker = args.next().unwrap();
            record_pid(&marker);
            thread::sleep(Duration::from_secs(30));
        }
        Some("read-eof") => {
            let marker = args.next().unwrap();
            record_pid(&marker);
            let mut bytes = Vec::new();
            io::stdin().read_to_end(&mut bytes).unwrap();
            println!("{}", bytes.len());
        }
        _ => process::exit(112),
    }
}
"#;

static NEXT_MARKER: AtomicU64 = AtomicU64::new(0);

pub struct Fixture {
    workspace: PathBuf,
    spawn: RunnerSpawn,
}

impl Fixture {
    pub fn run_program(&self) -> RunProgram {
        let scope = RunnerScope::new(vec![HELPER.to_owned()]).expect("helper name is valid");
        RunProgram::new(
            self.spawn.clone(),
            scope,
            Duration::from_secs(40),
            Arc::new(StaticCredentialSource::new(Vec::<(String, String)>::new())),
        )
    }

    pub fn marker(&self, label: &str) -> PathBuf {
        self.workspace.join(format!(
            "{label}-{}",
            NEXT_MARKER.fetch_add(1, Ordering::Relaxed)
        ))
    }
}

pub fn fixture() -> Option<&'static Fixture> {
    static FIXTURE: OnceLock<Option<Fixture>> = OnceLock::new();
    FIXTURE.get_or_init(build_fixture).as_ref()
}

fn build_fixture() -> Option<Fixture> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after epoch")
        .as_nanos();
    let root = env::temp_dir().join(format!("saya-c01b-{}-{stamp}", std::process::id()));
    let workspace = root.join("workspace");
    let programs = root.join("programs");
    fs::create_dir_all(&workspace).expect("workspace is created");
    // The sandbox profile allows only the canonicalised root (spike §4.2
    // measured that a /private/var rule denies a write via the /var alias),
    // so every marker path handed to a child must be the resolved one.
    let workspace = fs::canonicalize(&workspace).expect("workspace path canonicalises");
    fs::create_dir_all(&programs).expect("program directory is created");
    fs::set_permissions(&workspace, fs::Permissions::from_mode(0o700))
        .expect("workspace is private");
    fs::set_permissions(&programs, fs::Permissions::from_mode(0o700))
        .expect("program directory is private");

    let source = root.join("c01b-helper.rs");
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
                "C01b skipped: host sandbox is not proven\n{}",
                report.render()
            );
            return None;
        }
    };
    Some(Fixture { workspace, spawn })
}

pub async fn wait_for_file(path: &Path) -> u32 {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(value) = fs::read_to_string(path)
                && let Ok(pid) = value.parse()
            {
                break pid;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("real sandboxed command wrote its PID marker")
}

pub fn process_is_alive(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}
