use super::*;

/// A per-test scratch root: created, used, removed on drop.
pub(super) struct TempRoot(PathBuf);

impl TempRoot {
    pub(super) fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("saya-sandbox-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("test temp root must be creatable");
        Self(path)
    }

    pub(super) fn canonical(&self) -> PathBuf {
        fs::canonicalize(&self.0).expect("test temp root must canonicalise")
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A loopback listener whose accept thread hands every landed payload to the
/// test — genuinely accepting, per the starved-listener lesson: a
/// listener that never accepts turns TCP timeouts into false denials.
#[cfg(target_os = "macos")]
pub(super) struct AcceptingListener {
    pub(super) port: u16,
    received: mpsc::Receiver<String>,
}

#[cfg(target_os = "macos")]
impl AcceptingListener {
    pub(super) fn bind() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener must bind");
        let port = listener.local_addr().expect("listener addr").port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(mut stream) => {
                        let mut buf = [0u8; 64];
                        let n = std::io::Read::read(&mut stream, &mut buf).unwrap_or(0);
                        if tx
                            .send(String::from_utf8_lossy(&buf[..n]).into_owned())
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Self { port, received: rx }
    }

    pub(super) fn receive(&self, timeout: Duration) -> Option<String> {
        self.received.recv_timeout(timeout).ok()
    }
}

/// A bounded child run for the test canaries: a wedged sandboxed child is
/// killed at the bound instead of hanging the suite.
#[cfg(target_os = "macos")]
pub(super) fn bounded(cmd: &mut Command) -> std::process::Output {
    use std::io::Read as _;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("canary child must spawn");
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = pipe.read_to_end(&mut stdout);
                }
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_end(&mut stderr);
                }
                return std::process::Output {
                    status,
                    stdout,
                    stderr,
                };
            }
            Ok(None) => {
                if start.elapsed() > Duration::from_secs(30) {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("canary child wedged past the wall bound");
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => panic!("canary child failed to run: {e}"),
        }
    }
}

/// The composition root's registration rule, exercised at the sandbox seam: the
/// shell tool joins the episode's tool universe only when the sandbox handed
/// over a spawn configuration. The gate is this arm — there is no other way
/// a confined tool may appear.
#[cfg(unix)]
pub(super) fn runner_tool_names(provision: &SandboxProvision) -> Vec<&'static str> {
    provision.spawn().map(|_| vec!["shell"]).unwrap_or_default()
}

/// One Unix probe-failure policy: the root exists at construction, is deleted,
/// and the probe `prepare` runs then fails on the real host surface — a
/// changed environment, never an assumption (hardening environments change
/// under the run).
#[cfg(unix)]
pub(super) fn policy_with_vanished_root() -> (RunSandbox, TempRoot) {
    let root = TempRoot::new("probe-fails");
    let canonical = root.canonical();
    let policy = RunSandbox::new([canonical.clone()], Vec::<(String, u16)>::new())
        .expect("a real, existing root constructs");
    fs::remove_dir_all(&canonical).expect("root removal");
    (policy, root)
}

/// The runner program directory for Unix probe tests.
#[cfg(unix)]
pub(super) fn runner_program_dir() -> PathBuf {
    PathBuf::from("/bin")
}

/// Prepares a proven spawn for the tests: a real root, the given net_allow,
/// and `/bin` as the canary program directory. The startup probe runs in
/// full; the tests only proceed when it proved the sandbox.
#[cfg(target_os = "macos")]
pub(super) fn proven_spawn(
    tag: &str,
    net_allow: Vec<(String, u16)>,
) -> (TempRoot, SandboxProvision) {
    let root = TempRoot::new(tag);
    let canonical = root.canonical();
    let policy = RunSandbox::new([canonical], net_allow).expect("a real, existing root constructs");
    let provision = policy
        .prepare(&PathBuf::from("/bin"))
        .expect("preparation itself must run");
    assert!(
        provision.report().proves_runner(),
        "the healthy-host probe must prove the runner:\n{}",
        provision.report().render()
    );
    (root, provision)
}
