use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use saya_harness::{
    runner::sandbox::{RunSandbox, RunnerSpawn, SandboxProvision},
    runner::{Credential, RunProgram, StaticCredentialSource},
};
use saya_types::{RunnerScope, SecretRef};

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

const HELPER_SOURCE: &str = r#"
use std::{
    env, fs,
    io::{self, Write},
    path::Path,
    process, thread,
    time::Duration,
};

const SPLIT_BENIGN_PREFIX: usize = 5000;

fn record_pid(path: &str) {
    fs::write(path, process::id().to_string()).unwrap();
}

fn flood_byte(index: usize, upper: bool) -> u8 {
    let base = if upper { b'A' } else { b'a' };
    base + (index % 26) as u8
}

fn benign<W: Write>(w: &mut W, total: usize) -> io::Result<()> {
    let mut buf = vec![0u8; 1024];
    let mut written = 0;
    while written < total {
        let n = buf.len().min(total - written);
        for (offset, byte) in buf.iter_mut().enumerate().take(n) {
            *byte = flood_byte(written + offset, false);
        }
        w.write_all(&buf[..n])?;
        written += n;
    }
    w.flush()
}

fn flood<W: Write>(w: &mut W, total: usize, upper: bool) -> io::Result<()> {
    let mut buf = vec![0u8; 8192];
    let mut written = 0;
    while written < total {
        let n = buf.len().min(total - written);
        for (offset, byte) in buf.iter_mut().enumerate().take(n) {
            *byte = flood_byte(written + offset, upper);
        }
        w.write_all(&buf[..n])?;
        written += n;
    }
    w.flush()
}

fn await_sync_marker(marker: &str) {
    // Bounded wait for the test to touch the sync marker file: without it,
    // the second half is never written, so a hung test cannot deadlock the
    // helper past its own deadline.
    for _ in 0..500 {
        if Path::new(marker).exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("flood") => {
            let stdout_mib: usize = args.next().unwrap().parse().unwrap();
            let stderr_mib: usize = args.next().unwrap().parse().unwrap();
            let mut out = io::BufWriter::new(io::stdout());
            flood(&mut out, stdout_mib * 1024 * 1024, false).unwrap();
            let mut err = io::BufWriter::new(io::stderr());
            flood(&mut err, stderr_mib * 1024 * 1024, true).unwrap();
        }
        Some("drip") => {
            let lines: usize = args.next().unwrap().parse().unwrap();
            for i in 0..lines {
                println!("c02-drip {i}");
                io::stdout().flush().unwrap();
                thread::sleep(Duration::from_millis(20));
            }
        }
        Some("hold") => {
            record_pid(&args.next().unwrap());
            thread::sleep(Duration::from_secs(30));
        }
        Some("echo-credential") => {
            let name = args.next().unwrap();
            let value = env::var(&name).unwrap_or_default();
            println!("{name}={value}");
        }
        Some("split-credential") => {
            // Emits a wide benign prefix, then the env name and the first
            // half of the credential value, flushes, records the first-half
            // marker, and waits for the test's release marker before
            // writing the second half and a benign tail: the credential
            // value is guaranteed to straddle the pump's emission cut, and
            // the marker tells the test the first write has landed before
            // it looks at the store.
            let name = args.next().unwrap();
            let value = env::var(&name).unwrap_or_default();
            let release = args.next().unwrap();
            let first_half_marker = args.next().unwrap();
            let mid = value.len() / 2;
            let mut out = io::stdout();
            benign(&mut out, SPLIT_BENIGN_PREFIX).unwrap();
            out.write_all(name.as_bytes()).unwrap();
            out.write_all(b"=").unwrap();
            out.write_all(value[..mid].as_bytes()).unwrap();
            out.flush().unwrap();
            fs::write(&first_half_marker, b"first").unwrap();
            await_sync_marker(&release);
            out.write_all(value[mid..].as_bytes()).unwrap();
            out.write_all(b"\n").unwrap();
            benign(&mut out, 512).unwrap();
            out.flush().unwrap();
            thread::sleep(Duration::from_millis(250));
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
    /// Directory holding the fixture's compiled helper executable.
    pub fn program_dir(&self) -> &Path {
        self.spawn.program_dir()
    }

    /// One `RunProgram` allowlisting the helper, carrying the fixture's one
    /// declared credential: only its resolved value can reach a child's
    /// environment, and only that value is what the redaction gate must catch.
    pub fn run_program(&self) -> RunProgram {
        let scope = RunnerScope::new(vec![HELPER.to_owned()]).expect("helper name is valid");
        let credential = Credential::new(
            "c02-token",
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
                CREDENTIAL_VALUE.to_owned(),
            )])),
        )
        .with_credentials(vec![credential])
    }

    /// A unique private path under the fixture workspace, for pid markers and
    /// per-test store roots alike.
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
    let root = env::temp_dir().join(format!("saya-c02-{}-{stamp}", std::process::id()));
    let workspace = root.join("workspace");
    let programs = root.join("programs");
    fs::create_dir_all(&workspace).expect("workspace is created");
    fs::create_dir_all(&programs).expect("program directory is created");
    fs::set_permissions(&workspace, fs::Permissions::from_mode(0o700))
        .expect("workspace is private");
    fs::set_permissions(&programs, fs::Permissions::from_mode(0o700))
        .expect("program directory is private");

    let source = root.join("c02-helper.rs");
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
                "C02 skipped: host sandbox is not proven\n{}",
                report.render()
            );
            return None;
        }
    };
    Some(Fixture { workspace, spawn })
}
