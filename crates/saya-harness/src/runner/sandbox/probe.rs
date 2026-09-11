//! The startup probe: measures, on the host it runs on and against the real
//! generator output for this policy, whether the runner's sandbox actually
//! confines — and reports what it found. Promoted from the M5-3 spike probe
//! (`tests/sandbox_probe.rs`, merged) into the gate [`RunSandbox::prepare`]
//! consumes; the spike's file remains the deeper measurement record.
//!
//! The probe records findings; it does not enforce anything itself. A host
//! that lacks a feature is a recorded result, never a failed build, and the
//! verdict is fail closed by construction ([`ProbeReport::proves_runner`]).
//! Every deny a required check counts must carry its EPERM evidence — on
//! macOS the literal `Operation not permitted` in the child's stderr, the
//! rule the spike learned the hard way (a TCP timeout is not a sandbox
//! denial). Every canary runs unprivileged, bounded in wall clock, with
//! captured output capped in bytes.
//!
//! Platform batteries: `probe_macos` (Seatbelt), `probe_linux` (Landlock +
//! namespaces, UNVERIFIED on any Linux host), and the explicit windows /
//! other-platform recorded results here. Shared plumbing lives in
//! `probe_support`.

use super::report::ProbeReport;

// `Check` is built only by the explicit windows/other recorded results
// here; the platform batteries build their checks in their own modules.
#[cfg(any(windows, not(any(target_os = "macos", target_os = "linux"))))]
use super::report::Check;

/// The probe's platform dispatch. Each platform's module records its own
/// required checks; the verdict reads only those.
pub(super) fn run(sb: &super::RunSandbox) -> ProbeReport {
    #[cfg(target_os = "macos")]
    return super::probe_macos::run(sb);
    #[cfg(target_os = "linux")]
    return super::probe_linux::run(sb);
    #[cfg(windows)]
    return windows_report(sb);
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    return other_report(sb);
}

/// Windows: an explicit recorded result, not an absence. Per U9 the runner
/// fails closed by construction there: this check can never pass, the spawn
/// configuration never exists, and the 3-OS matrix stays green with the
/// runner simply absent.
#[cfg(windows)]
fn windows_report(_sb: &super::RunSandbox) -> ProbeReport {
    ProbeReport::new(
        "windows",
        format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        vec![
            Check::info(
                "windows_probe_scope",
                "the probe records findings, it does not enforce anything; on Windows \
                 there is nothing to measure",
            ),
            Check::fail(
                "sandbox_available",
                true,
                "explicit recorded result: Windows has no OS sandbox primitive this \
                 design targets — no Seatbelt, no Landlock, no supported equivalent. \
                 Per U9 the runner fails closed by construction on Windows.",
            ),
        ],
    )
}

/// Any other platform (e.g. the BSDs): recorded, not absent.
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn other_report(_sb: &super::RunSandbox) -> ProbeReport {
    ProbeReport::new(
        std::env::consts::OS,
        format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        vec![Check::fail(
            "sandbox_available",
            true,
            format!(
                "explicit recorded result: this platform ({}/{}) has no measured sandbox \
                 path in this design; fail closed by construction",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        )],
    )
}
