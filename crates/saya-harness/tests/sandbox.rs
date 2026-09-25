//! The M5-3 sandbox module's integration tests — the fail-closed contract,
//! measured against the real `sandbox-exec` on this host where one exists.
//!
//! Six guarantees, one per red test the milestone demanded:
//! 1. Probe failure ⇒ the runner is not registered ⇒ a plan requesting the
//!    runner scope is refused. Asserted on the tool list and the typed plan
//!    refusal, never on a flag.
//! 2. A read outside `fs_roots` is denied, with `Operation not permitted` in
//!    stderr.
//! 3. A write outside `fs_roots` is denied; a write inside succeeds and the
//!    file exists afterwards.
//! 4. Egress to a port not in `net_allow` is denied with `Operation not
//!    permitted`; an allowed port connects — against a listener that
//!    actually calls `accept()` (the spike's starved-listener lesson).
//! 5. `#[cfg(windows)]`: profile construction refuses the unsupported root,
//!    so the runner is not registered.
//! 6. The generated profile is rejected by `sandbox-exec` if a placeholder
//!    was left unsubstituted — and because a *quoted* leftover is silently
//!    accepted (measured on this host: exit 0), the generator itself refuses
//!    leftovers.

use std::{fs, path::PathBuf};

#[cfg(unix)]
use std::{collections::VecDeque, sync::Mutex};
#[cfg(target_os = "macos")]
use std::{
    net::TcpListener,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

#[cfg(unix)]
use async_trait::async_trait;
#[cfg(unix)]
use saya_agent::{ChatMessage, ChatProvider, ChatRequest, ChatResponse, ProviderError};
#[cfg(unix)]
use saya_harness::engine::{PlanDriver, PlanError, PlanRejection, PlanRequest};
#[cfg(unix)]
use saya_harness::runner::sandbox::SandboxProvision;
use saya_harness::runner::sandbox::{RunSandbox, SandboxError};
#[cfg(unix)]
use saya_types::{Budgets, Capabilities, RunnerScope};

/// A per-test scratch root: created, used, removed on drop.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("saya-sandbox-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("test temp root must be creatable");
        Self(path)
    }

    fn canonical(&self) -> PathBuf {
        fs::canonicalize(&self.0).expect("test temp root must canonicalise")
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A loopback listener whose accept thread hands every landed payload to the
/// test — genuinely accepting, per the spike's starved-listener lesson: a
/// listener that never accepts turns TCP timeouts into false denials.
#[cfg(target_os = "macos")]
struct AcceptingListener {
    port: u16,
    received: mpsc::Receiver<String>,
}

#[cfg(target_os = "macos")]
impl AcceptingListener {
    fn bind() -> Self {
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

    fn receive(&self, timeout: Duration) -> Option<String> {
        self.received.recv_timeout(timeout).ok()
    }
}

/// A bounded child run for the test canaries: a wedged sandboxed child is
/// killed at the bound instead of hanging the suite.
#[cfg(target_os = "macos")]
fn bounded(cmd: &mut Command) -> std::process::Output {
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

/// The composition root's registration rule, exercised at M5-3's seam: the
/// runner tool joins the episode's tool universe only when the sandbox
/// handed over a spawn configuration. M5-4 supplies the definition; the gate
/// is this arm — there is no other way a runner tool may appear.
#[cfg(unix)]
fn runner_tool_names(provision: &SandboxProvision) -> Vec<&'static str> {
    provision
        .spawn()
        .map(|_| vec!["run_program"])
        .unwrap_or_default()
}

/// Builds the approved scopes a run would carry, with the runner allowed.
#[cfg(unix)]
fn approved_runner_scopes() -> Capabilities {
    let mut caps = Capabilities::default();
    caps.runner =
        Some(RunnerScope::new(vec!["echo".to_owned()]).expect("echo is a shaped program name"));
    caps
}

/// A scripted planner provider: serves its answers in order and records
/// every request, so the refusal loop's attempt count is pinned.
#[cfg(unix)]
struct ScriptedPlanner {
    script: Mutex<VecDeque<String>>,
    requests: Mutex<Vec<ChatRequest>>,
}

#[cfg(unix)]
impl ScriptedPlanner {
    fn new(script: Vec<String>) -> Self {
        Self {
            script: Mutex::new(script.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[cfg(unix)]
#[async_trait]
impl ChatProvider for ScriptedPlanner {
    fn name(&self) -> &str {
        "scripted-planner"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        match self.script.lock().unwrap().pop_front() {
            Some(text) => Ok(ChatResponse::new(ChatMessage::text("assistant", text))),
            None => panic!("planner script exhausted"),
        }
    }
}

/// The plan JSON one step needs: a goal, the runner-scoped capabilities, and
/// nothing else the contract would refuse for shape.
#[cfg(unix)]
fn runner_plan_json() -> String {
    r#"{"steps": [{"goal": "run the approved program", "capabilities": {"runner": {"programs": ["echo"]}}, "budget": null, "expects": [], "endpoint": null}]}"#
        .to_owned()
}

/// One Unix probe-failure policy: the root exists at construction, is deleted,
/// and the probe `prepare` runs then fails on the real host surface — a
/// changed environment, never an assumption (spike §9.3: hardening
/// environments change under the run).
#[cfg(unix)]
fn policy_with_vanished_root() -> (RunSandbox, TempRoot) {
    let root = TempRoot::new("probe-fails");
    let canonical = root.canonical();
    let policy = RunSandbox::new([canonical.clone()], Vec::<(String, u16)>::new())
        .expect("a real, existing root constructs");
    fs::remove_dir_all(&canonical).expect("root removal");
    (policy, root)
}

/// The runner program directory for Unix probe tests.
#[cfg(unix)]
fn runner_program_dir() -> PathBuf {
    PathBuf::from("/bin")
}

/// Test 1 — the fail-closed chain: probe failure ⇒ the runner is not
/// registered (no spawn configuration exists, so the tool universe carries
/// no runner tool) ⇒ a plan requesting the runner scope is refused with the
/// ordinary needs-approval outcome after the bounded attempts.
#[cfg(unix)]
#[tokio::test]
async fn probe_failure_leaves_the_runner_unregistered_and_refuses_runner_scopes() {
    let (policy, _root) = policy_with_vanished_root();
    let provision = policy
        .prepare(&runner_program_dir())
        .expect("preparation itself must run");

    // The probe ran and did not prove the runner.
    let report = provision.report();
    assert!(
        !report.proves_runner(),
        "a policy whose roots vanished must not be proven:\n{}",
        report.render()
    );
    assert!(
        !report.failed_required().is_empty(),
        "the report must name what failed:\n{}",
        report.render()
    );

    // The tool list: with no spawn configuration the runner tool cannot join
    // the universe the composition root builds.
    assert!(
        runner_tool_names(&provision).is_empty(),
        "an unproven sandbox registers no runner tool"
    );
    assert!(provision.spawn().is_none());

    // The plan surface: a plan requesting the runner scope is refused with
    // the ordinary needs-approval outcome, after the bounded re-prompts.
    let approved = approved_runner_scopes();
    let stripped = provision.plan_capabilities(&approved);
    assert!(
        stripped.runner.is_none(),
        "the runner scope must be absent from what plan validation sees"
    );

    let planner = ScriptedPlanner::new(vec![runner_plan_json(); 3]);
    let driver = PlanDriver::new(
        &planner,
        PlanRequest {
            model: "mock-model".into(),
            run_goal: "run the approved program".into(),
        },
    );
    let error = driver
        .propose(&stripped, &Budgets::default())
        .await
        .unwrap_err();
    assert!(
        matches!(
            &error,
            PlanError::Exhausted {
                attempts: 3,
                last: PlanRejection::NeedsApproval { step: 0, scopes: _ },
            }
        ),
        "the third refusal must be the typed needs-approval outcome: {error:?}"
    );
    assert_eq!(
        planner.request_count(),
        3,
        "exactly the bound was spent: three proposals, then the refusal"
    );
}

// ---------------------------------------------------------------------------
// macOS-only enforcement tests: these run real sandbox-exec children under
// the real generated profile.
// ---------------------------------------------------------------------------

/// Prepares a proven spawn for the tests: a real root, the given net_allow,
/// and `/bin` as the canary program directory. The startup probe runs in
/// full; the tests only proceed when it proved the sandbox.
#[cfg(target_os = "macos")]
fn proven_spawn(tag: &str, net_allow: Vec<(String, u16)>) -> (TempRoot, SandboxProvision) {
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

/// Test 2 — a read outside `fs_roots` is denied, with the EPERM evidence in
/// stderr; the unsandboxed control read succeeds, so the denial is
/// attributed to the sandbox.
#[cfg(target_os = "macos")]
#[test]
fn a_read_outside_fs_roots_is_denied_with_eperm() {
    let (_root, provision) = proven_spawn("read-outside", vec![]);
    let spawn = provision.spawn().expect("proven");

    let control = bounded(Command::new("/bin/cat").arg("/private/etc/hosts"));
    assert!(
        control.status.success(),
        "the unsandboxed control read must succeed, else the deny is unattributed: \
         {control:?}"
    );

    let output = bounded(
        spawn
            .command(Path::new("/bin/cat"))
            .arg("/private/etc/hosts"),
    );
    assert!(
        !output.status.success(),
        "the sandboxed read was not denied"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Operation not permitted"),
        "the deny must carry EPERM evidence, got: {stderr}"
    );
    assert!(
        output.stdout.is_empty(),
        "the denied read must deliver nothing: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// Test 3 — writes outside the roots are denied (EPERM, nothing on disk); a
/// write inside succeeds and the file exists afterwards, verified
/// unsandboxed.
#[cfg(target_os = "macos")]
#[test]
fn writes_outside_are_denied_inside_succeed() {
    let (root, provision) = proven_spawn("write", vec![]);
    let spawn = provision.spawn().expect("proven");
    let inside = root.canonical();
    let outside = TempRoot::new("write-outside");
    let outside_path = outside.canonical();

    let control = bounded(Command::new("/bin/mkdir").arg(outside_path.join("control-write")));
    assert!(
        control.status.success(),
        "the unsandboxed control write must succeed: {control:?}"
    );

    let denied = bounded(
        spawn
            .command(Path::new("/bin/mkdir"))
            .arg(outside_path.join("denied-write")),
    );
    assert!(
        !denied.status.success(),
        "the sandboxed write was not denied"
    );
    let stderr = String::from_utf8_lossy(&denied.stderr);
    assert!(
        stderr.contains("Operation not permitted"),
        "the deny must carry EPERM evidence, got: {stderr}"
    );
    assert!(
        !outside_path.join("denied-write").exists(),
        "a denied write must leave nothing on disk"
    );

    let allowed = bounded(
        spawn
            .command(Path::new("/bin/mkdir"))
            .arg(inside.join("after-probe-write")),
    );
    assert!(
        allowed.status.success(),
        "the sandboxed write inside fs_roots must succeed: {:?}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    assert!(
        inside.join("after-probe-write").is_dir(),
        "the written directory must exist afterwards, unsandboxed"
    );
}

/// Test 4 — egress to a port not in `net_allow` is denied with EPERM; the
/// allowed port connects, and every landing is observed on a listener that
/// calls `accept()`.
#[cfg(target_os = "macos")]
#[test]
fn egress_to_unallowed_port_is_denied_allowed_port_connects() {
    let allowed = AcceptingListener::bind();
    let control_listener = AcceptingListener::bind();
    let (_root, provision) = proven_spawn("egress", vec![("127.0.0.1".to_owned(), allowed.port)]);
    let spawn = provision.spawn().expect("proven");
    let connect = |port: u16| format!("exec 3<>/dev/tcp/127.0.0.1/{port} && printf ping >&3");

    // Control: the same connect unsandboxed must land on the accepting
    // listener, so the sandboxed results below are attributed.
    let control = bounded(
        Command::new("/bin/bash")
            .arg("-c")
            .arg(connect(control_listener.port)),
    );
    assert!(
        control.status.success(),
        "the unsandboxed control connect must succeed: {control:?}"
    );
    assert_eq!(
        control_listener.receive(Duration::from_secs(2)).as_deref(),
        Some("ping"),
        "the control connect must land on the accepting listener"
    );

    // Allowed endpoint: connects, and the listener actually accepts.
    let attempt = bounded(
        spawn
            .command(Path::new("/bin/bash"))
            .arg("-c")
            .arg(connect(allowed.port)),
    );
    assert!(
        attempt.status.success(),
        "the sandboxed connect to the allowed port must succeed: {:?}",
        String::from_utf8_lossy(&attempt.stderr)
    );
    assert_eq!(
        allowed.receive(Duration::from_secs(2)),
        Some("ping".to_owned()),
        "the allowed connect must land on the accepting listener"
    );

    // Denied endpoint: a port no rule names is refused with EPERM — the
    // literal text, never a timeout mistaken for a denial.
    let denied = bounded(
        spawn
            .command(Path::new("/bin/bash"))
            .arg("-c")
            .arg(connect(control_listener.port)),
    );
    assert!(
        !denied.status.success(),
        "the sandboxed connect was not denied"
    );
    let stderr = String::from_utf8_lossy(&denied.stderr);
    assert!(
        stderr.contains("Operation not permitted"),
        "the egress deny must carry EPERM evidence, got: {stderr}"
    );
}

/// Test 6 — a half-rendered profile must not silently become a permissive
/// one: the generator refuses leftovers (the necessary guard), sandbox-exec
/// rejects an unquoted leftover at parse, and a quoted leftover is accepted
/// by sandbox-exec and matches nothing — the measured fail-open mode that
/// makes the generator-side guard mandatory.
#[cfg(target_os = "macos")]
#[test]
fn a_half_rendered_profile_is_refused_by_the_generator_and_by_sandbox_exec() {
    use saya_harness::runner::sandbox::macos;

    // Injection refusal at construction: a root whose text could alter the
    // profile language is refused, never escaped (escaping is unmeasured).
    let paren = TempRoot::new("paren");
    let inside_paren = paren.canonical().join("sub(close-paren)");
    fs::create_dir_all(&inside_paren).expect("the paren-named dir must be creatable");
    assert!(
        matches!(
            RunSandbox::new([inside_paren], Vec::<(String, u16)>::new()),
            Err(SandboxError::RootNotSafeForProfile { .. })
        ),
        "a root containing profile-language characters must be refused at construction"
    );

    // The generator emits no leftover placeholders, and says what it does
    // not cover: the net_allow host component is enforced in-process only —
    // in the header when there are no entries, and on every rule when there
    // are.
    let root = TempRoot::new("gen");
    let sb = RunSandbox::new([root.canonical()], Vec::<(String, u16)>::new())
        .expect("policy constructs");
    let profile = macos::seatbelt_profile(&sb, &[PathBuf::from("/bin")])
        .expect("a valid policy generates a profile");
    assert!(
        !profile.contains('{'),
        "the generated profile must carry no placeholder: {profile}"
    );
    assert!(
        profile.contains("enforced in-process by the M3-1 fetch policy"),
        "the profile must say what it does not cover (net_allow hosts): {profile}"
    );
    let sb_with_net = RunSandbox::new([root.canonical()], [("127.0.0.1".to_owned(), 8443)])
        .expect("policy with loopback entry constructs");
    let profile_with_net = macos::seatbelt_profile(&sb_with_net, &[PathBuf::from("/bin")])
        .expect("a valid policy generates a profile");
    assert!(
        profile_with_net.contains("host enforced in-process only"),
        "each net rule must say the host is enforced in-process only: {profile_with_net}"
    );

    // An unquoted leftover is rejected at parse (measured: exit 65,
    // "unbound variable").
    let unquoted = format!("{profile}\n(allow process-exec (subpath {{SAYA_LEFTOVER}}))\n");
    let out = bounded(
        Command::new("/usr/bin/sandbox-exec")
            .arg("-p")
            .arg(&unquoted)
            .arg("/bin/echo")
            .arg("x")
            .current_dir(root.canonical()),
    );
    assert_eq!(
        out.status.code(),
        Some(65),
        "an unquoted leftover must be rejected at parse: {out:?}"
    );

    // A quoted leftover is silently accepted and matches nothing (measured
    // on this host: exit 0) — the fail-open mode. The generator's scan is
    // what catches it, so the scan must reject this exact text.
    let quoted = format!("{profile}\n(allow file-read* (subpath \"{{SAYA_LEFT}}\"))\n");
    let out = bounded(
        Command::new("/usr/bin/sandbox-exec")
            .arg("-p")
            .arg(&quoted)
            .arg("/bin/echo")
            .arg("x")
            .current_dir(root.canonical()),
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "the measured fail-open mode: sandbox-exec accepts a quoted leftover and it \
         matches nothing — which is why the generator refuses placeholders itself"
    );
    assert!(
        macos::has_leftover_placeholder(&quoted),
        "the generator-side scan must refuse the text sandbox-exec silently accepts"
    );
}

/// Construction refuses what the platform's sandbox cannot express — per
/// platform, at construction, never silently narrowed.
#[cfg(not(windows))]
#[test]
fn construction_refuses_the_unexpressible() {
    let root = TempRoot::new("construct");
    let canonical = root.canonical();
    assert!(
        matches!(
            RunSandbox::new(Vec::<PathBuf>::new(), Vec::<(String, u16)>::new()),
            Err(SandboxError::NoRoots)
        ),
        "a policy with no roots is refused"
    );
    assert!(
        matches!(
            RunSandbox::new([canonical.clone()], [("127.0.0.1".to_owned(), 0)]),
            Err(SandboxError::NetPortInvalid { .. })
        ),
        "port 0 is not an enforceable egress declaration"
    );
    assert!(
        matches!(
            RunSandbox::new([canonical.clone()], [("example.com".to_owned(), 443)]),
            Err(SandboxError::NetHostNotExpressible { .. })
        ),
        "a named remote host is not expressible by any platform sandbox in this design"
    );
}

/// Test 5 — Windows has no measured profile language. Windows path spelling
/// is therefore rejected before a policy is created, and the runner remains
/// unavailable by construction.
#[cfg(windows)]
#[test]
fn windows_refuses_a_profile_root_and_never_creates_a_runner_policy() {
    let root = TempRoot::new("windows");
    let canonical = root.canonical();
    assert!(
        matches!(
            RunSandbox::new([canonical], Vec::<(String, u16)>::new()),
            Err(SandboxError::RootNotSafeForProfile { .. })
        ),
        "Windows has no supported sandbox profile path; policy construction must fail closed"
    );
}

/// The registration evidence, written where a human reads it: the probe
/// report and the exact profile the run would spawn under.
#[cfg(target_os = "macos")]
#[test]
fn the_probe_report_is_written_for_the_run() {
    let (_root, provision) = proven_spawn("report", vec![]);
    let dir = std::env::var("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let _ = fs::write(
        dir.join("sandbox-startup-report.txt"),
        provision.report().render(),
    );
    let spawn = provision.spawn().expect("proven");
    if let Some(profile) = spawn.profile() {
        let _ = fs::write(dir.join("sandbox-startup-profile.sb"), profile);
    }
}
