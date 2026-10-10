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

pub const HELPER: &str = "saya-c01a-helper";
const HELPER_SOURCE: &str = r#"
use std::{env, fs, process, thread, time::Duration};

fn write_pid(path: &str) {
    fs::write(path, process::id().to_string()).unwrap();
}

fn hold(seconds: u64) {
    thread::sleep(Duration::from_secs(seconds));
}

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("hold") => {
            let marker = args.next().unwrap();
            let seconds = args.next().unwrap().parse().unwrap();
            write_pid(&marker);
            hold(seconds);
        }
        Some("fork-hold") => unsafe {
            let parent_marker = args.next().unwrap();
            let child_marker = args.next().unwrap();
            match fork() {
                0 => {
                    write_pid(&child_marker);
                    hold(30);
                    process::exit(0);
                }
                pid if pid > 0 => {
                    write_pid(&parent_marker);
                    hold(30);
                }
                _ => process::exit(111),
            }
        },
        Some("exit") => {
            let marker = args.next().unwrap();
            let code = args.next().unwrap().parse().unwrap();
            write_pid(&marker);
            process::exit(code);
        }
        _ => process::exit(112),
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn fork() -> i32;
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

    pub fn marker_pair(&self, label: &str) -> (PathBuf, PathBuf) {
        (
            self.marker(&format!("{label}-parent")),
            self.marker(&format!("{label}-child")),
        )
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
    let root = env::temp_dir().join(format!("saya-c01a-{}-{stamp}", std::process::id()));
    let workspace = root.join("workspace");
    let programs = root.join("programs");
    fs::create_dir_all(&workspace).expect("workspace is created");
    fs::create_dir_all(&programs).expect("program directory is created");
    fs::set_permissions(&workspace, fs::Permissions::from_mode(0o700))
        .expect("workspace is private");
    fs::set_permissions(&programs, fs::Permissions::from_mode(0o700))
        .expect("program directory is private");

    let source = root.join("c01a-helper.rs");
    let binary = programs.join(HELPER);
    let blocked = programs.join("blocked-helper");
    fs::write(&source, HELPER_SOURCE).expect("native test helper source writes");
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let status = Command::new(rustc)
        .arg("--edition=2021")
        .arg("-o")
        .arg(&binary)
        .arg(&source)
        .status()
        .expect("the active Rust toolchain compiles the native test helper");
    assert!(
        status.success(),
        "the native test helper compiles: {status}"
    );
    fs::copy(&binary, &blocked).expect("blocked allowlist trap is staged");
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700))
        .expect("blocked allowlist trap is executable");

    let policy = RunSandbox::new([workspace.clone()], Vec::new())
        .expect("the workspace is a valid sandbox root")
        .with_process_fork("C01a supervised cleanup fixture uses a real forked child")
        .expect("the bounded test reason is safe");
    let provision = policy
        .prepare(&programs)
        .expect("the host sandbox probe completes");
    let spawn = match provision {
        SandboxProvision::Proven { spawn, .. } => spawn,
        SandboxProvision::Refused(report) => {
            eprintln!(
                "C01a skipped: host sandbox is not proven\n{}",
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
    .expect("the real sandboxed process wrote its start marker")
}

pub fn process_is_alive(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub async fn wait_group_gone(pgid: u32) -> bool {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if unsafe { libc::killpg(pgid as libc::pid_t, 0) } != 0
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                break true;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or(false)
}
