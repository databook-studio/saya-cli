//! The macOS canary infrastructure: the process-unique temp targets, the
//! genuinely-accepting loopback listener, the sandboxed-child helpers, and
//! the EPERM-evidence deny rule. The battery itself (`probe_macos`) composes
//! these into the required checks.
//!
//! A deny only counts when the operation failed, the failure carries the
//! literal `Operation not permitted` in stderr, and — where a filesystem
//! target is involved — nothing landed on disk. A listener bound with
//! `listen(1)` that never accepts turns a timeout into a false denial, so
//! the listener here always calls `accept()`.

use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

use super::macos::SANDBOX_EXEC;
use super::probe_support::{Captured, run_bounded};
use super::report::Check;

pub(super) const EPERM_EVIDENCE: &str = "Operation not permitted";

/// The canary programs' directory: `/bin` held every probe child (echo,
/// mkdir, cat, bash) on the measured host.
pub(super) const CANARY_PROGRAM_DIR: &str = "/bin";

/// How many connects must land on the allowed endpoint for the check to
/// count. Measurement showed 10/10; at startup three all-landing attempts
/// keep the probe bounded while still demanding reliability.
pub(super) const ALLOWED_CONNECT_ATTEMPTS: usize = 3;

/// A process-unique suffix for every path the probe creates: concurrent
/// probes in one process (a test harness, parallel runs) must never share a
/// target, or one run's cleanup deletes another run's evidence.
pub(super) fn unique(tag: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{tag}-{}-{n}", std::process::id())
}

/// A temp directory for the canaries' outside-root targets; removed on drop.
pub(super) struct TempRoot(PathBuf);

impl TempRoot {
    pub(super) fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(unique(format!("saya-sbx-probe-{tag}").as_str()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("probe temp root must be creatable");
        Self(path)
    }

    pub(super) fn canonical(&self) -> PathBuf {
        fs::canonicalize(&self.0).expect("probe temp root must canonicalise")
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A loopback listener whose accept thread hands every received payload to
/// the battery — a genuinely accepting endpoint, per the deny-evidence rule.
pub(super) struct ProbeListener {
    pub(super) port: u16,
    received: mpsc::Receiver<String>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ProbeListener {
    pub(super) fn bind() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let (tx, rx) = mpsc::sync_channel(8);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if stream
                            .set_read_timeout(Some(Duration::from_millis(100)))
                            .is_err()
                        {
                            break;
                        }
                        let mut buf = [0u8; 64];
                        let n = std::io::Read::read(&mut stream, &mut buf).unwrap_or(0);
                        if let Err(mpsc::TrySendError::Disconnected(_)) =
                            tx.try_send(String::from_utf8_lossy(&buf[..n]).into_owned())
                        {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            port,
            received: rx,
            stop,
            thread: Some(thread),
        })
    }

    /// Waits for one landed connection, bounded.
    pub(super) fn receive(&self, timeout: Duration) -> Option<String> {
        self.received.recv_timeout(timeout).ok()
    }
}

impl Drop for ProbeListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Set on the process the test below re-executes. Its value is the file that
/// process writes once its run has completed, and its presence tells the test
/// that it is that process.
#[cfg(test)]
const CHILD_MARKER: &str = "SAYA_TEST_LISTENER_DROP_MARKER";
#[cfg(test)]
const CHILD_TEST: &str =
    "runner::sandbox::macos_canary::listener_drop_closes_its_port_even_with_a_silent_connection";

/// Dropping a listener closes its port even while a connection to it is silent.
///
/// The check runs in a process of its own. macOS cannot create a socket with
/// close-on-exec set atomically, so a child that a parallel test forks while
/// this listener's socket is being created inherits it and holds the port open
/// past the drop. The final connect then succeeds for a reason that is not the
/// listener's. Nothing forks in the isolated process.
#[test]
fn listener_drop_closes_its_port_even_with_a_silent_connection() {
    use std::{process::Stdio, time::Instant};
    if let Some(marker) = std::env::var_os(CHILD_MARKER) {
        listener_drop_closes_its_port();
        fs::write(marker, b"ok").expect("record the completed child run");
        return;
    }
    let root = TempRoot::new("listener-drop");
    let marker = root.canonical().join("completed");
    let mut child = Command::new(std::env::current_exe().expect("test executable resolves"))
        .args(["--exact", CHILD_TEST, "--nocapture"])
        .env(CHILD_MARKER, &marker)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("isolated listener test starts");
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("poll the isolated listener test") {
            Some(status) => break status,
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("isolated listener test exceeded 30 seconds and was reaped");
            }
        }
    };
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = std::io::Read::read_to_string(&mut pipe, &mut stderr);
    }
    assert!(
        status.success(),
        "isolated listener test failed: {status}\n{stderr}"
    );
    assert_eq!(
        fs::read(&marker).expect("the exact child test must run to completion"),
        b"ok"
    );
}

#[cfg(test)]
fn listener_drop_closes_its_port() {
    use std::{
        net::{SocketAddr, TcpStream},
        time::Instant,
    };
    let listener = ProbeListener::bind().unwrap();
    let address: SocketAddr = format!("127.0.0.1:{}", listener.port).parse().unwrap();
    let _silent = TcpStream::connect_timeout(&address, Duration::from_secs(1)).unwrap();
    let start = Instant::now();
    drop(listener);
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_err());
}

pub(super) fn connect_script(port: u16) -> String {
    format!("exec 3<>/dev/tcp/127.0.0.1/{port} && printf ping >&3")
}

pub(super) fn sandboxed(profile: &str, cwd: &Path, program: &str, args: &[&str]) -> Captured {
    let mut c = Command::new(SANDBOX_EXEC);
    c.arg("-p").arg(profile).arg(program);
    c.args(args);
    c.current_dir(cwd);
    run_bounded(&mut c)
}

pub(super) fn plain(program: &str, args: &[&str]) -> Captured {
    let mut c = Command::new(program);
    c.args(args);
    run_bounded(&mut c)
}

/// A deny only counts when the operation failed, the failure says EPERM, and
/// — where a filesystem target is involved — nothing landed on disk.
pub(super) fn expect_deny(
    captured: &Captured,
    what: &'static str,
    required: bool,
    absent: Option<&Path>,
) -> Check {
    let evidence = captured.stderr.contains(EPERM_EVIDENCE);
    let nothing_landed = absent.is_none_or(|p| !p.exists());
    if !captured.exited_ok() && evidence && nothing_landed {
        Check::pass(what, required, captured.summary())
    } else if captured.exited_ok() {
        Check::fail(
            what,
            required,
            format!(
                "NOT DENIED — the sandboxed operation succeeded; the sandbox did not \
                 enforce this deny.\n{}",
                captured.summary()
            ),
        )
    } else {
        Check::fail(
            what,
            required,
            format!(
                "the operation failed, but without {EPERM_EVIDENCE:?} in stderr or \
                 without the expected absence on disk the cause is unattributed:\n{}",
                captured.summary()
            ),
        )
    }
}

/// The egress half of the battery: the probe policy's rules name the probe
/// listener (appended) and whatever the policy declared; the control
/// listener's port names no rule. If it collided with a declared port the
/// check would prove nothing — bind another until it does not.
pub(super) fn egress_checks(
    checks: &mut Vec<Check>,
    sb: &super::RunSandbox,
    profile: &str,
    root: &Path,
    listener: &ProbeListener,
) {
    let mut control_listener = ProbeListener::bind().expect("probe control listener");
    while sb
        .net_allow()
        .iter()
        .any(|(h, p)| super::validate::seatbelt_host(h).is_some() && *p == control_listener.port)
    {
        control_listener = ProbeListener::bind().expect("probe control listener");
    }
    let control_connect = plain("/bin/bash", &["-c", &connect_script(control_listener.port)]);
    checks.push(super::probe_support::check_exited_ok(
        &control_connect,
        "control_net_connect",
        true,
        |_| {
            format!(
                "unsandboxed connect landed: {:?}",
                control_listener.receive(Duration::from_millis(500))
            )
        },
    ));

    checks.push(expect_deny(
        &sandboxed(
            profile,
            root,
            "/bin/bash",
            &["-c", &connect_script(control_listener.port)],
        ),
        "egress_denied_other_port",
        true,
        None,
    ));

    let mut landed = 0usize;
    for _ in 0..ALLOWED_CONNECT_ATTEMPTS {
        let attempt = sandboxed(
            profile,
            root,
            "/bin/bash",
            &["-c", &connect_script(listener.port)],
        );
        if attempt.exited_ok() && listener.receive(Duration::from_millis(500)).is_some() {
            landed += 1;
        }
    }
    checks.push(if landed == ALLOWED_CONNECT_ATTEMPTS {
        Check::pass(
            "egress_allowed_endpoint",
            true,
            format!(
                "{landed}/{ALLOWED_CONNECT_ATTEMPTS} sandboxed connects to the allowed \
                 port landed on the accepting listener"
            ),
        )
    } else {
        Check::fail(
            "egress_allowed_endpoint",
            true,
            format!(
                "{landed}/{ALLOWED_CONNECT_ATTEMPTS} sandboxed connects landed — the \
                 seatbelt allow for net_allow is not reliable on this host"
            ),
        )
    });
}
