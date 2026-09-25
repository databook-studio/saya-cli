//! M5-3 spike probe — measures, on the host it runs on, which OS sandboxing is
//! actually available to the future runner, and records what it found. It is a
//! probe, not a gate: a host that lacks a feature is a recorded result, never a
//! failed build.
//!
//! The verdict is fail closed by construction: [`ProbeReport::proves_runner`]
//! is true only when every *required* check on this platform passed. Windows
//! has no sandbox primitive this design targets, so its probe records that
//! explicitly; Linux is not proven here until the `landlock` dependency lands
//! and the enforcement probe runs on a Linux host, so its report says so. The
//! runner-registration rule (M5-3) consumes exactly this verdict: no proof, no
//! runner tool.
//!
//! All commands run unprivileged, with a wall-clock bound, and with captured
//! output capped in bytes. The full report is echoed to stdout (visible with
//! `--nocapture`) and written to `$CARGO_TARGET_TMPDIR/sandbox-probe-report.txt`.

use std::{fmt::Write as _, fs, path::PathBuf};

// Unix-only helpers: `uname`, bounded command capture, and the sandbox-exec
// and namespace probes. Windows builds record the fail-closed result without
// any of these.
#[cfg(not(windows))]
use std::{
    io::Read as _,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[cfg(not(windows))]
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
#[cfg(not(windows))]
const DETAIL_CAP: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Passed,
    Failed,
}

struct Check {
    name: &'static str,
    required: bool,
    status: Status,
    detail: String,
}

impl Check {
    fn passed(name: &'static str, required: bool, detail: impl Into<String>) -> Self {
        Self {
            name,
            required,
            status: Status::Passed,
            detail: detail.into(),
        }
    }

    fn failed(name: &'static str, required: bool, detail: impl Into<String>) -> Self {
        Self {
            name,
            required,
            status: Status::Failed,
            detail: detail.into(),
        }
    }

    fn info(name: &'static str, detail: impl Into<String>) -> Self {
        Self::passed(name, false, detail)
    }
}

struct ProbeReport {
    platform: &'static str,
    kernel: String,
    checks: Vec<Check>,
}

impl ProbeReport {
    /// The fail-closed rule, stated as code: the runner may be registered on
    /// this host only if every required check passed. No checks at all, or any
    /// required check not passed, means not proven. An optional check failing
    /// is context, never proof.
    fn proves_runner(&self) -> bool {
        !self.checks.is_empty()
            && self
                .checks
                .iter()
                .all(|c| !c.required || c.status == Status::Passed)
    }

    fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "platform: {}", self.platform);
        let _ = writeln!(out, "kernel: {}", self.kernel);
        let _ = writeln!(
            out,
            "verdict: {}",
            if self.proves_runner() {
                "runner PROVEN on this host"
            } else {
                "runner NOT proven on this host — fail closed"
            }
        );
        for c in &self.checks {
            let status = match c.status {
                Status::Passed => "PASS",
                Status::Failed => "FAIL",
            };
            let required = if c.required { "required" } else { "optional" };
            let _ = writeln!(out, "\n[{status}] {required} check {}", c.name);
            let _ = write!(out, "{}", indent(&c.detail));
        }
        out
    }
}

fn indent(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let _ = writeln!(out, "    {line}");
    }
    out
}

#[cfg(not(windows))]
struct Captured {
    spawn_error: Option<String>,
    timed_out: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[cfg(not(windows))]
impl Captured {
    fn exited_ok(&self) -> bool {
        !self.timed_out && self.spawn_error.is_none() && self.code == Some(0)
    }

    fn summary(&self) -> String {
        if let Some(e) = &self.spawn_error {
            return format!("spawn failed: {e}");
        }
        let exit = if self.timed_out {
            "TIMEOUT".to_owned()
        } else {
            format!(
                "exit {}",
                self.code.map_or("?".to_owned(), |c| c.to_string())
            )
        };
        let mut out = format!("exit: {exit}");
        if !self.stdout.is_empty() {
            let _ = write!(out, "\nstdout:\n{}", indent(&self.stdout));
        }
        if !self.stderr.is_empty() {
            let _ = write!(out, "\nstderr:\n{}", indent(&self.stderr));
        }
        out
    }
}

#[cfg(not(windows))]
fn run_bounded(cmd: &mut Command) -> Captured {
    let blank = Captured {
        spawn_error: None,
        timed_out: false,
        code: None,
        stdout: String::new(),
        stderr: String::new(),
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("LC_ALL", "C")
        .env("LANG", "C");
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Captured {
                spawn_error: Some(e.to_string()),
                ..blank
            };
        }
    };
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if start.elapsed() > COMMAND_TIMEOUT {
                    let _ = child.kill();
                    timed_out = true;
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Captured {
                    spawn_error: Some(e.to_string()),
                    ..blank
                };
            }
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    Captured {
        spawn_error: None,
        timed_out,
        code: status.and_then(|s| s.code()),
        stdout: cap(&stdout),
        stderr: cap(&stderr),
    }
}

#[cfg(not(windows))]
fn cap(text: &str) -> String {
    if text.len() <= DETAIL_CAP {
        return text.to_owned();
    }
    let mut end = DETAIL_CAP;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… (truncated at {DETAIL_CAP} bytes)", &text[..end])
}

#[cfg(not(windows))]
fn uname_line() -> String {
    let out = run_bounded(Command::new("uname").arg("-a"));
    if out.exited_ok() {
        out.stdout.trim().to_owned()
    } else {
        format!(
            "{} {} (uname unavailable: {})",
            std::env::consts::OS,
            std::env::consts::ARCH,
            out.summary()
        )
    }
}

#[cfg(target_os = "macos")]
const REQUIRED_CHECK_NAMES: &[&str] = &[
    "sandbox_exec_present",
    "bogus_profile_rejected",
    "deny_default_denies_exec",
    "control_write_outside_root",
    "write_denied_outside_root",
    "write_allowed_in_root",
    "control_read_outside_roots",
    "read_denied_outside_roots",
    "control_net_connect",
    "net_denied_without_allow",
];

#[cfg(target_os = "linux")]
const REQUIRED_CHECK_NAMES: &[&str] = &[
    "landlock_abi",
    "user_network_namespace_unprivileged",
    "landlock_enforcement",
];

#[cfg(windows)]
const REQUIRED_CHECK_NAMES: &[&str] = &["sandbox_available"];

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
const REQUIRED_CHECK_NAMES: &[&str] = &["sandbox_available"];

/// macOS: Seatbelt via `sandbox-exec` with a generated profile. The probe
/// measures the binary's presence, that it *rejects* an invalid profile (a
/// no-op `sandbox-exec` would make every later check meaningless), that
/// `deny default` actually denies, that write is allowed only inside
/// `fs_roots`, that read outside the roots is denied, and that outbound
/// network is denied without an allow rule. Every deny result must carry
/// EPERM evidence in the child's stderr — a failure from some other cause
/// would be an unmeasured claim, and this probe does not make those.
///
/// The selective-egress side (`net_allow`) is measured with a reliability
/// batch: the chosen allow rule must land EVERY attempted connect (10/10),
/// and a port the rule does not name must be denied. On the measured host the
/// `localhost` host-form is the only one Seatbelt accepts beside `*` — a raw
/// IP or a DNS name is rejected at parse — and the accepted rule is enforced
/// port-exact. If the batch ever fails, the verdict fails closed: the runner
/// must refuse net_allow policies on that host until something reliable is
/// proven.
#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::{net::TcpListener, path::Path};

    const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
    const EPERM_EVIDENCE: &str = "Operation not permitted";

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("saya-sandbox-probe-{tag}-{}", std::process::id()));
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

    fn net_checks(checks: &mut Vec<Check>, root: &Path) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("probe listener");
        let port = listener.local_addr().expect("listener addr").port();
        let sb = RunSandbox {
            fs_roots: vec![root.to_path_buf()],
            net_allow: vec![("127.0.0.1".to_owned(), port)],
        };

        let mut chosen: Option<NetRule> = None;
        let mut variant_notes = String::new();
        for rule in [
            NetRule::TcpRawHost,
            NetRule::TcpLocalhostExact,
            NetRule::IpLocalhostExact,
            NetRule::TcpAnyPort,
        ] {
            let profile = seatbelt_profile(&sb, Some(rule));
            let probe = sandboxed(
                SANDBOX_EXEC,
                &profile,
                root,
                "/bin/echo",
                &["saya-net-rule-ok"],
            );
            if probe.exited_ok() {
                let _ = writeln!(
                    variant_notes,
                    "ACCEPTED (profile compiles) {}: {}",
                    rule.name(),
                    probe.summary()
                );
                if chosen.is_none() {
                    chosen = Some(rule);
                }
            } else {
                let _ = writeln!(
                    variant_notes,
                    "REJECTED {}:\n{}",
                    rule.name(),
                    indent(&probe.summary())
                );
            }
        }
        checks.push(Check::info(
            "net_rule_variant_accepted",
            format!(
                "chosen: {}",
                chosen.map_or("none of the tested variants".to_owned(), |r| r
                    .name()
                    .to_owned())
            ) + &variant_notes,
        ));

        let control_listener = TcpListener::bind("127.0.0.1:0").expect("control listener");
        let control_port = control_listener.local_addr().expect("control addr").port();
        let control = plain_connect(control_port);
        checks.push(if control.exited_ok() {
            Check::passed("control_net_connect", true, control.summary())
        } else {
            Check::failed(
                "control_net_connect",
                true,
                format!(
                    "unsandboxed connect to a loopback listener failed, so the sandboxed \
                     egress results below are unattributed:\n{}",
                    control.summary()
                ),
            )
        });

        let no_net = RunSandbox {
            fs_roots: vec![root.to_path_buf()],
            net_allow: Vec::new(),
        };
        let profile_no_net = seatbelt_profile(&no_net, None);
        checks.push(expect_deny(
            &sandboxed(
                SANDBOX_EXEC,
                &profile_no_net,
                root,
                "/bin/bash",
                &["-c", &connect_script(port)],
            ),
            "net_denied_without_allow",
            true,
            None,
        ));

        checks.push(match chosen {
            None => Check::failed(
                "net_allowed_with_rule",
                false,
                format!("no tested network-allow rule syntax was accepted:\n{variant_notes}"),
            ),
            Some(rule) => {
                let profile = seatbelt_profile(&sb, Some(rule));
                let attempts = 10;
                let mut landed = 0usize;
                let mut exits_ok = 0usize;
                let mut first_denial = None;
                for _ in 0..attempts {
                    let got = sandboxed(
                        SANDBOX_EXEC,
                        &profile,
                        root,
                        "/bin/bash",
                        &["-c", &connect_script(port)],
                    );
                    if got.exited_ok() {
                        exits_ok += 1;
                    } else if first_denial.is_none() {
                        first_denial = Some(got.summary());
                    }
                    if receive_on_listener(&listener, Duration::from_millis(500)).is_some() {
                        landed += 1;
                    }
                }
                let reliable = exits_ok == attempts && landed == attempts;
                let detail = format!(
                    "rule kind {}: of {attempts} attempted connects to the allowed \
                     port, {exits_ok} exited cleanly and {landed} landed on the \
                     listener; the seatbelt allow for net_allow on this host is \
                     therefore {}.",
                    rule.name(),
                    if reliable {
                        "reliable (every connect landed)"
                    } else {
                        "NOT reliable — selective egress is unproven, and the runner \
                         must refuse net_allow policies here"
                    }
                ) + &first_denial
                    .map_or_else(String::new, |s| format!("\nfirst denial:\n{}", indent(&s)));
                if reliable {
                    Check::passed("net_allowed_with_rule", false, detail)
                } else {
                    Check::failed("net_allowed_with_rule", false, detail)
                }
            }
        });

        checks.push(match chosen {
            None => Check::failed(
                "net_other_port_denied",
                false,
                format!(
                    "no rule syntax was accepted, so port-exactness is unmeasured:\n\
                     {variant_notes}"
                ),
            ),
            Some(rule) => {
                let profile = seatbelt_profile(&sb, Some(rule));
                let attempts = 5;
                let mut denied = 0usize;
                let mut last = None;
                for _ in 0..attempts {
                    let got = sandboxed(
                        SANDBOX_EXEC,
                        &profile,
                        root,
                        "/bin/bash",
                        &["-c", &connect_script(control_port)],
                    );
                    let was_denied = !got.exited_ok() && got.stderr.contains(EPERM_EVIDENCE);
                    if was_denied {
                        denied += 1;
                    }
                    last = Some(got);
                }
                let detail = format!(
                    "rule kind {}: connecting to a port the rule does not name was \
                     denied in {denied}/{attempts} attempts (port-exact enforcement).",
                    rule.name(),
                );
                if denied == attempts {
                    Check::passed("net_other_port_denied", false, detail)
                } else {
                    Check::failed(
                        "net_other_port_denied",
                        false,
                        detail
                            + &last.map_or_else(String::new, |c| {
                                format!("\nlast attempt:\n{}", indent(&c.summary()))
                            }),
                    )
                }
            }
        });
    }

    pub(super) fn run() -> ProbeReport {
        let uname = uname_line();
        let mut checks = vec![Check::info("host_uname", uname.clone())];
        let run_root = TempRoot::new("run-root");
        let outside_root = TempRoot::new("outside-root");
        let root = run_root.canonical();
        let outside = outside_root.canonical();

        checks.push(if Path::new(SANDBOX_EXEC).exists() {
            Check::passed(
                "sandbox_exec_present",
                true,
                format!("{SANDBOX_EXEC} exists on this host"),
            )
        } else {
            Check::failed(
                "sandbox_exec_present",
                true,
                format!("{SANDBOX_EXEC} missing; no Seatbelt path on this host"),
            )
        });

        let bogus = sandboxed(
            SANDBOX_EXEC,
            "not a sandbox profile at all",
            &root,
            "/bin/echo",
            &["saya-probe-bogus"],
        );
        checks.push(if !bogus.exited_ok() {
            Check::passed("bogus_profile_rejected", true, bogus.summary())
        } else {
            Check::failed(
                "bogus_profile_rejected",
                true,
                format!(
                    "sandbox-exec accepted an invalid profile — it cannot be trusted to \
                     parse profiles at all:\n{}",
                    bogus.summary()
                ),
            )
        });

        checks.push(expect_deny(
            &sandboxed(
                SANDBOX_EXEC,
                "(version 1)\n(deny default)\n",
                &root,
                "/bin/echo",
                &["x"],
            ),
            "deny_default_denies_exec",
            true,
            None,
        ));

        let control_write = plain(
            "/bin/mkdir",
            &[&outside.join("control-write").to_string_lossy()],
        );
        checks.push(if control_write.exited_ok() {
            Check::passed("control_write_outside_root", true, control_write.summary())
        } else {
            Check::failed(
                "control_write_outside_root",
                true,
                format!(
                    "unsandboxed mkdir into the outside dir failed, so the sandboxed \
                     write-deny result below is unattributable:\n{}",
                    control_write.summary()
                ),
            )
        });

        let profile_no_net = seatbelt_profile(
            &RunSandbox {
                fs_roots: vec![root.clone()],
                net_allow: Vec::new(),
            },
            None,
        );
        checks.push(expect_deny(
            &sandboxed(
                SANDBOX_EXEC,
                &profile_no_net,
                &root,
                "/bin/mkdir",
                &[&outside.join("denied-write").to_string_lossy()],
            ),
            "write_denied_outside_root",
            true,
            Some(&outside.join("denied-write")),
        ));

        let allowed = sandboxed(
            SANDBOX_EXEC,
            &profile_no_net,
            &root,
            "/bin/mkdir",
            &[&root.join("allowed-write").to_string_lossy()],
        );
        let dir_created = root.join("allowed-write").is_dir();
        checks.push(if allowed.exited_ok() && dir_created {
            Check::passed(
                "write_allowed_in_root",
                true,
                format!(
                    "sandboxed mkdir inside fs_roots worked; the directory exists \
                     unsandboxed afterwards:\n{}",
                    allowed.summary()
                ),
            )
        } else {
            Check::failed(
                "write_allowed_in_root",
                true,
                format!(
                    "sandboxed write inside fs_roots did not work (directory exists: \
                     {dir_created}):\n{}",
                    allowed.summary()
                ),
            )
        });

        let control_read = plain("/bin/cat", &["/private/etc/hosts"]);
        checks.push(if control_read.exited_ok() {
            Check::passed("control_read_outside_roots", true, control_read.summary())
        } else {
            Check::failed(
                "control_read_outside_roots",
                true,
                format!(
                    "unsandboxed cat of /private/etc/hosts failed, so the sandboxed \
                     read-deny result below is unattributable:\n{}",
                    control_read.summary()
                ),
            )
        });
        checks.push(expect_deny(
            &sandboxed(
                SANDBOX_EXEC,
                &profile_no_net,
                &root,
                "/bin/cat",
                &["/private/etc/hosts"],
            ),
            "read_denied_outside_roots",
            true,
            None,
        ));

        net_checks(&mut checks, &root);

        ProbeReport {
            platform: "macos",
            kernel: uname,
            checks,
        }
    }
}

/// Linux: measures the Landlock ABI version the kernel reports (raw
/// `landlock_create_ruleset` version query, syscall 444), whether an
/// unprivileged user+network namespace can be created (`fork` + `unshare`,
/// because `unshare(CLONE_NEWUSER)` is refused in a multithreaded process and
/// the test harness is threaded), and records the kernel's Landlock-related
/// state. It deliberately does NOT probe enforcement: proving a Landlock
/// ruleset denies needs the `landlock` dependency, which this spike must not
/// add, and a Linux host. The enforcement check is therefore recorded as
/// failed — Linux cannot be proven by this probe, which is exactly the
/// fail-closed posture.
#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    fn errno_text(errno: i32) -> String {
        std::io::Error::from_raw_os_error(errno).to_string()
    }

    fn read_record(path: &str) -> String {
        fs::read_to_string(path).unwrap_or_else(|e| format!("absent ({e})"))
    }

    fn landlock_abi() -> Result<u32, i32> {
        let r = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<libc::c_void>(),
                0usize,
                1u32,
            )
        };
        if r == -1 {
            Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
        } else {
            Ok(u32::try_from(r).unwrap_or(u32::MAX))
        }
    }

    unsafe fn child_fail(fd: libc::c_int) -> ! {
        unsafe {
            let errno = *libc::__errno_location();
            libc::write(fd, std::ptr::from_ref(&errno).cast::<libc::c_void>(), 4);
            libc::close(fd);
            libc::_exit(1);
        }
    }

    fn unprivileged_user_net_ns() -> Result<(), String> {
        unsafe {
            let mut fds: [libc::c_int; 2] = [0; 2];
            if libc::pipe(fds.as_mut_ptr()) != 0 {
                return Err(format!("pipe: {}", errno_text(*libc::__errno_location())));
            }
            match libc::fork() {
                -1 => Err(format!("fork: {}", errno_text(*libc::__errno_location()))),
                0 => {
                    libc::close(fds[0]);
                    if libc::unshare(libc::CLONE_NEWUSER) != 0 {
                        child_fail(fds[1]);
                    }
                    if libc::unshare(libc::CLONE_NEWNET) != 0 {
                        child_fail(fds[1]);
                    }
                    libc::close(fds[1]);
                    libc::_exit(0);
                }
                pid => {
                    libc::close(fds[1]);
                    let mut buf = [0u8; 4];
                    let mut got = 0usize;
                    loop {
                        let n = libc::read(
                            fds[0],
                            buf.as_mut_ptr().add(got).cast::<libc::c_void>(),
                            4 - got,
                        );
                        if n < 0 {
                            if *libc::__errno_location() == libc::EINTR {
                                continue;
                            }
                            break;
                        }
                        if n == 0 {
                            break;
                        }
                        got += n as usize;
                        if got == 4 {
                            break;
                        }
                    }
                    libc::close(fds[0]);
                    let mut status: libc::c_int = 0;
                    loop {
                        if libc::waitpid(pid, &mut status, 0) == -1
                            && *libc::__errno_location() == libc::EINTR
                        {
                            continue;
                        }
                        break;
                    }
                    let errno = (got == 4).then(|| i32::from_ne_bytes(buf));
                    let exited = (status & 0x7f) == 0;
                    let code = ((status >> 8) & 0xff) as i32;
                    let signal = status & 0x7f;
                    if errno.is_none() && exited && code == 0 {
                        Ok(())
                    } else if let Some(e) = errno {
                        Err(format!(
                            "unshare(CLONE_NEWUSER) + unshare(CLONE_NEWNET) failed: {}",
                            errno_text(e)
                        ))
                    } else {
                        Err(format!(
                            "probe child did not exit cleanly (exited={exited}, \
                             code={code}, signal={signal})"
                        ))
                    }
                }
            }
        }
    }

    pub(super) fn run() -> ProbeReport {
        let uname = uname_line();
        let mut checks = vec![Check::info("host_uname", uname.clone())];

        checks.push(match landlock_abi() {
            Ok(v) => Check::passed(
                "landlock_abi",
                true,
                format!(
                    "landlock_create_ruleset(NULL, 0, LANDLOCK_CREATE_RULESET_VERSION) \
                     returned ABI version {v}"
                ),
            ),
            Err(e) => Check::failed(
                "landlock_abi",
                true,
                format!(
                    "Landlock version query failed (errno {e}: {}): this kernel does not \
                     expose Landlock to unprivileged callers",
                    errno_text(e)
                ),
            ),
        });

        let lsm = read_record("/sys/kernel/security/lsm");
        checks.push(Check::info(
            "lsm_list",
            format!(
                "landlock listed in /sys/kernel/security/lsm: {}\ncontent: {}",
                lsm.contains("landlock"),
                lsm.trim_end()
            ),
        ));
        checks.push(Check::info(
            "user_max_user_namespaces",
            read_record("/proc/sys/user/max_user_namespaces")
                .trim_end()
                .to_owned(),
        ));

        checks.push(match unprivileged_user_net_ns() {
            Ok(()) => Check::passed(
                "user_network_namespace_unprivileged",
                true,
                "a forked, unprivileged child ran unshare(CLONE_NEWUSER) then \
                 unshare(CLONE_NEWNET); both succeeded",
            ),
            Err(e) => Check::failed(
                "user_network_namespace_unprivileged",
                true,
                format!(
                    "unprivileged user+network namespace creation failed: {e} — the \
                     netns egress mechanism is unavailable on this host"
                ),
            ),
        });

        checks.push(Check::failed(
            "landlock_enforcement",
            true,
            "not probed in this slice: proving a Landlock ruleset denies requires the \
             `landlock` dependency (which this spike must not add) and a Linux host to \
             run on. The checks above are availability measurements only; this probe \
             can never prove the runner on Linux, so Linux fails closed until the \
             implementation slice lands.",
        ));

        ProbeReport {
            platform: "linux",
            kernel: uname,
            checks,
        }
    }
}

/// Windows: an explicit recorded result, not an absence. No probe is possible,
/// and none is needed — per U9 the runner fails closed by construction there.
#[cfg(windows)]
mod platform {
    use super::*;

    pub(super) fn run() -> ProbeReport {
        ProbeReport {
            platform: "windows",
            kernel: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            checks: vec![
                Check::info(
                    "windows_probe_scope",
                    "the probe records findings, it does not enforce anything; on \
                     Windows there is nothing to measure",
                ),
                Check::failed(
                    "sandbox_available",
                    true,
                    "explicit recorded result: Windows has no OS sandbox primitive this \
                     design targets — no Seatbelt, no Landlock, no supported equivalent. \
                     Per U9 the runner fails closed by construction on Windows: this check \
                     can never pass, the runner tool is never registered, and the 3-OS CI \
                     matrix stays green with the runner simply absent.",
                ),
            ],
        }
    }
}

/// Any other platform (e.g. the BSDs): recorded, not absent.
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod platform {
    use super::*;

    pub(super) fn run() -> ProbeReport {
        ProbeReport {
            platform: "other-unix",
            kernel: uname_line(),
            checks: vec![Check::failed(
                "sandbox_available",
                true,
                format!(
                    "explicit recorded result: this platform ({}/{}) has no measured \
                     sandbox path in this design; fail closed by construction",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ),
            )],
        }
    }
}

#[test]
fn probe_records_host_sandbox_capabilities() {
    let report = platform::run();
    let text = report.render();
    println!("{text}");

    let dir = std::env::var("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or(std::env::temp_dir());
    let path = dir.join("sandbox-probe-report.txt");
    fs::write(&path, &text).expect("probe report must be writable");
    println!("report written to {}", path.display());

    assert!(
        !report.checks.is_empty(),
        "a probe must always record checks"
    );
    for name in REQUIRED_CHECK_NAMES {
        assert!(
            report.checks.iter().any(|c| c.name == *name),
            "required check {name} missing from the report"
        );
    }
    let all_required_passed = report
        .checks
        .iter()
        .all(|c| !c.required || c.status == Status::Passed);
    assert_eq!(report.proves_runner(), all_required_passed);
}

#[test]
fn verdict_fails_closed_by_construction() {
    let report = |checks: Vec<Check>| ProbeReport {
        platform: "synthetic",
        kernel: String::new(),
        checks,
    };

    assert!(
        report(vec![
            Check::passed("required-check", true, String::new()),
            Check::passed("optional-check", false, String::new()),
        ])
        .proves_runner()
    );
    assert!(
        report(vec![
            Check::passed("required-check", true, String::new()),
            Check::failed("optional-check", false, String::new()),
        ])
        .proves_runner()
    );
    assert!(
        !report(vec![
            Check::passed("required-check", true, String::new()),
            Check::failed("required-check-2", true, String::new()),
        ])
        .proves_runner()
    );
    assert!(!report(Vec::new()).proves_runner());
}
