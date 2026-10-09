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

#[path = "sandbox_probe/report.rs"]
mod report;
use self::report::{Check, ProbeReport, Status, indent};
#[cfg(not(windows))]
#[path = "sandbox_probe/command.rs"]
mod command;
#[cfg(not(windows))]
use self::command::*;

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
#[path = "sandbox_probe/macos.rs"]
mod platform;

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
#[path = "sandbox_probe/linux.rs"]
mod platform;

/// Windows: an explicit recorded result, not an absence. No probe is possible,
/// and none is needed — per U9 the runner fails closed by construction there.
#[cfg(windows)]
#[path = "sandbox_probe/windows.rs"]
mod platform;

/// Any other platform (e.g. the BSDs): recorded, not absent.
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
#[path = "sandbox_probe/other.rs"]
mod platform;

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
