//! The battery's fixed infrastructure: the rustc-compiled helper binary
//! with its argv/env/daemon/exit modes, the per-test staging directory, and
//! the host config shape every case runs against. Its behaviors live beside
//! it: `argv` (byte-exact argv delivery), `env` (the child environment),
//! `path_resolution` (resolution and its typed refusals), `timeout_group`
//! (timeouts and group kill), and `cwd` (the bound root as the child's cwd).

#[path = "host_battery/argv.rs"]
mod argv;
#[path = "host_battery/cwd.rs"]
mod cwd;
#[path = "host_battery/env.rs"]
mod env;
#[path = "host_battery/path_resolution.rs"]
mod path_resolution;
#[path = "host_battery/timeout_group.rs"]
mod timeout_group;

#[path = "../support/exec_ready.rs"]
mod exec_ready;

use std::{fs, os::unix::fs::PermissionsExt as _, path::PathBuf, sync::OnceLock, time::Duration};

use saya_agent::CancellationToken;
use saya_harness::{
    host::{HostCommand, HostConfig},
    runner::ProgramOutcome,
};

/// The battery's one helper, compiled with the toolchain running the
/// tests: an argv dumper (hex per element), an env dumper, a forking
/// daemonizer, and an exit-code setter.
const HELPER_SOURCE: &str = r#"
use std::io::Write;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("");
    match mode {
        "argv" => {
            println!("argc={}", args.len());
            for (index, arg) in args.iter().enumerate() {
                let hex: String = arg.bytes().map(|b| format!("{b:02x}")).collect();
                println!("arg{index}={hex}");
            }
        }
        "env" => {
            let mut vars: Vec<(String, String)> = std::env::vars().collect();
            vars.sort();
            for (name, value) in vars {
                println!("{name}={value}");
            }
        }
        "fork-daemon" => unsafe {
            if fork() == 0 {
                if fork() == 0 {
                    sleep(60);
                    std::process::exit(0);
                }
                std::process::exit(0);
            }
            sleep(60);
        },
        "exit" => {
            let code: i32 = args.get(2).and_then(|c| c.parse().ok()).unwrap_or(0);
            std::process::exit(code);
        }
        _ => {
            eprintln!("unknown mode");
            std::process::exit(2);
        }
    }
    let _ = std::io::stdout().flush();
}

extern "C" {
    fn fork() -> i32;
    fn sleep(seconds: u32);
}
"#;

fn helper() -> &'static PathBuf {
    static HELPER: OnceLock<PathBuf> = OnceLock::new();
    HELPER.get_or_init(|| {
        // Beside the per-test directories, so staging can hard-link it.
        let base = std::env::temp_dir();
        let dir = base.join(format!("saya-host-helper-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("helper directory must be creatable");
        let source = dir.join("saya-host-helper.rs");
        let binary = dir.join("saya-host-helper");
        fs::write(&source, HELPER_SOURCE).expect("helper source must be written");
        let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
        let status = std::process::Command::new(rustc)
            .arg("--edition=2021")
            .arg("-o")
            .arg(&binary)
            .arg(&source)
            .status()
            .expect("rustc must be runnable — it is the toolchain running this test");
        assert!(
            status.success(),
            "the battery helper must compile: {status}"
        );
        run_once(&binary);
        binary
    })
}

/// macOS vets an executable the first time it runs, one at a time for the
/// whole machine, and every test execs its own hard link of the helper. Run
/// once here, the helper is vetted before any test is timed; without it each
/// test's first exec queues behind that vetting and, under load, the system
/// can kill the exec outright. A run the system kills proves nothing about the
/// helper, so it is repeated a few times before it fails the battery.
fn run_once(binary: &std::path::Path) {
    let mut last = None;
    for _ in 0..3 {
        let status = std::process::Command::new(binary)
            .args(["exit", "0"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("the battery helper must start");
        if status.success() {
            return;
        }
        last = Some(status);
    }
    panic!("the battery helper did not complete its first run: {last:?}");
}

fn leak(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("saya-host-test-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("battery dir must be creatable");
    fs::canonicalize(&path).expect("battery dir must canonicalise")
}

/// Stages the helper under `dir` as `name` with an exec bit, returning
/// the dir for PATH construction.
fn stage(dir: &std::path::Path, name: &str) -> PathBuf {
    let target = dir.join(name);
    // A hard link to the finished helper never opens a file for writing, so
    // exec cannot hit ETXTBSY through a descriptor another thread's fork
    // inherited; across filesystems a copy plus a readiness wait does the job.
    let _ = fs::remove_file(&target);
    if fs::hard_link(helper(), &target).is_err() {
        fs::copy(helper(), &target).expect("the helper must stage");
    }
    let mut perms = fs::metadata(&target).expect("staged").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&target, perms).expect("exec bit must set");
    exec_ready::wait_until_exec_ready(&target);
    dir.to_path_buf()
}

/// Runs the call and returns the outcome of the first run in which the helper
/// started. macOS can kill an exec outright while it vets the file, and under
/// load a test cannot prevent that: the outcome is a SIGKILL with no timeout,
/// no cancellation and nothing printed, and nothing in this crate sent it. The
/// kill says nothing about the property under test, so such a run is repeated
/// a few times; every assertion applies to a run in which the helper started.
async fn run_started(
    command: &HostCommand,
    config: &HostConfig,
    timeout_seconds: Option<u64>,
) -> ProgramOutcome {
    let mut last = None;
    for _ in 0..3 {
        let outcome = command
            .run(config, timeout_seconds, &CancellationToken::new())
            .await
            .expect("the run itself must complete");
        if !killed_at_exec(&outcome) {
            return outcome;
        }
        last = Some(outcome);
    }
    panic!("the system killed the helper at exec on every attempt: {last:?}");
}

fn killed_at_exec(outcome: &ProgramOutcome) -> bool {
    outcome.signal == Some(9)
        && !outcome.killed_by_timeout
        && !outcome.cancelled
        && outcome.stdout.text.is_empty()
        && outcome.stderr.text.is_empty()
}

fn config(path: &str, extra_env: &[(&str, &str)]) -> HostConfig {
    HostConfig::new(path, std::env::temp_dir(), Duration::from_secs(600))
        .expect("shaped")
        .with_extra_env(
            extra_env
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
        )
        .expect("shaped")
}
