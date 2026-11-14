const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
const EPERM_EVIDENCE: &str = "Operation not permitted";

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("saya-sandbox-probe-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("probe temp root must be creatable");
        Self(path)
    }

    fn canonical(&self) -> PathBuf {
        fs::canonicalize(&self.0).expect("temp root must canonicalise")
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct RunSandbox {
    fs_roots: Vec<PathBuf>,
    net_allow: Vec<(String, u16)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NetRule {
    /// The policy host verbatim — measured REJECTED: Seatbelt accepts
    /// only `*` or `localhost` in the remote filter's host position.
    TcpRawHost,
    /// Policy loopback IP mapped to Seatbelt's `localhost` — measured
    /// accepted and enforced port-exact.
    TcpLocalhostExact,
    /// Compiles, but measured to block the connection silently.
    IpLocalhostExact,
    /// Compiles, but measured to block the connection silently.
    TcpAnyPort,
}

impl NetRule {
    fn name(self) -> &'static str {
        match self {
            Self::TcpRawHost => "tcp raw host",
            Self::TcpLocalhostExact => "tcp localhost exact",
            Self::IpLocalhostExact => "ip localhost exact",
            Self::TcpAnyPort => "tcp any port",
        }
    }

    fn clause(self, host: &str, port: u16) -> String {
        let remote = match self {
            Self::TcpRawHost => format!("(remote tcp \"{host}:{port}\")"),
            Self::TcpLocalhostExact => format!("(remote tcp \"localhost:{port}\")"),
            Self::IpLocalhostExact => format!("(remote ip \"localhost:{port}\")"),
            Self::TcpAnyPort => format!("(remote tcp \"*:{port}\")"),
        };
        format!("(allow network-outbound {remote})\n")
    }
}

fn seatbelt_profile(sb: &RunSandbox, net: Option<NetRule>) -> String {
    let mut p = String::from("(version 1)\n(deny default)\n");
    p.push_str("(allow process-exec (subpath \"/bin\"))\n");
    p.push_str("(allow file-read-data (literal \"/\"))\n");
    for root in &sb.fs_roots {
        let _ = writeln!(p, "(allow file-read* (subpath \"{}\"))", root.display());
        let _ = writeln!(p, "(allow file-write* (subpath \"{}\"))", root.display());
    }
    for (host, port) in &sb.net_allow {
        let rule = net.expect("net_allow entries require a rule kind");
        p.push_str(&rule.clause(host, *port));
    }
    p
}

fn sandboxed(exec: &str, profile: &str, cwd: &Path, program: &str, args: &[&str]) -> Captured {
    let mut c = Command::new(exec);
    c.arg("-p").arg(profile).arg(program);
    c.args(args);
    c.current_dir(cwd);
    run_bounded(&mut c)
}

fn plain(program: &str, args: &[&str]) -> Captured {
    let mut c = Command::new(program);
    c.args(args);
    run_bounded(&mut c)
}

/// A deny only counts when the operation failed, the failure says EPERM,
/// and — where a filesystem target is involved — nothing landed on disk.
fn expect_deny(
    captured: &Captured,
    what: &'static str,
    required: bool,
    absent: Option<&Path>,
) -> Check {
    let evidence = captured.stderr.contains(EPERM_EVIDENCE);
    let nothing_landed = absent.is_none_or(|p| !p.exists());
    if !captured.exited_ok() && evidence && nothing_landed {
        Check::passed(what, required, captured.summary())
    } else if captured.exited_ok() {
        Check::failed(
            what,
            required,
            format!(
                "NOT DENIED — the sandboxed operation succeeded; the profile did not \
                 enforce this deny.\n{}",
                captured.summary()
            ),
        )
    } else {
        Check::failed(
            what,
            required,
            format!(
                "the operation failed, but without {EPERM_EVIDENCE:?} in stderr or \
                 without the expected absence on disk, the cause is unattributed:\n{}",
                captured.summary()
            ),
        )
    }
}

fn connect_script(port: u16) -> String {
    format!("exec 3<>/dev/tcp/127.0.0.1/{port} && printf ping >&3")
}

fn plain_connect(port: u16) -> Captured {
    let mut c = Command::new("/bin/bash");
    c.arg("-c").arg(connect_script(port));
    run_bounded(&mut c)
}

fn receive_on_listener(listener: &TcpListener, timeout: Duration) -> Option<String> {
    listener.set_nonblocking(true).ok()?;
    let start = Instant::now();
    while start.elapsed() < timeout {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut buf = [0u8; 64];
                let n = stream.read(&mut buf).unwrap_or(0);
                return Some(String::from_utf8_lossy(&buf[..n]).into_owned());
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    }
    None
}
