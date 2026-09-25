//! The probe report: the recorded result of measuring which OS sandboxing
//! actually works on the host it runs on, and the fail-closed verdict the
//! runner registration consumes.
//!
//! Promoted from the M5-3 spike probe (`tests/sandbox_probe.rs`, merged):
//! a host that lacks a feature is a recorded result, never a failed build,
//! and the verdict is fail closed by construction — [`ProbeReport::proves_runner`]
//! is true only when every *required* check on this platform passed. An
//! optional check failing is recorded context, never proof; a platform the
//! probe cannot measure records that explicitly rather than being absent.

use std::fmt::Write as _;

/// One check the probe recorded.
#[derive(Debug)]
pub struct Check {
    name: &'static str,
    required: bool,
    status: CheckStatus,
    detail: String,
}

impl Check {
    /// The check's name, as it appears in the rendered report.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Whether the verdict required this check.
    pub fn required(&self) -> bool {
        self.required
    }

    /// Whether the check passed.
    pub fn passed(&self) -> bool {
        self.status == CheckStatus::Passed
    }

    /// The recorded evidence — the summary the check attached.
    pub fn detail(&self) -> &str {
        &self.detail
    }

    pub(crate) fn pass(name: &'static str, required: bool, detail: impl Into<String>) -> Self {
        Self {
            name,
            required,
            status: CheckStatus::Passed,
            detail: detail.into(),
        }
    }

    pub(crate) fn fail(name: &'static str, required: bool, detail: impl Into<String>) -> Self {
        Self {
            name,
            required,
            status: CheckStatus::Failed,
            detail: detail.into(),
        }
    }

    pub(crate) fn info(name: &'static str, detail: impl Into<String>) -> Self {
        Self::pass(name, false, detail)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckStatus {
    Passed,
    Failed,
}

impl CheckStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Passed => "PASS",
            Self::Failed => "FAIL",
        }
    }
}

/// What the startup probe measured on this host. Consumed by
/// [`crate::runner::sandbox::SandboxProvision`]: no proof, no runner.
#[derive(Debug)]
pub struct ProbeReport {
    platform: &'static str,
    kernel: String,
    checks: Vec<Check>,
}

impl ProbeReport {
    pub(crate) fn new(platform: &'static str, kernel: String, checks: Vec<Check>) -> Self {
        Self {
            platform,
            kernel,
            checks,
        }
    }

    /// The platform the probe ran on (`macos`, `linux`, `windows`, ...).
    pub fn platform(&self) -> &'static str {
        self.platform
    }

    /// The kernel or OS identity the probe recorded.
    pub fn kernel(&self) -> &str {
        &self.kernel
    }

    /// Every check the probe recorded, in the order it ran them.
    pub fn checks(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter()
    }

    /// The fail-closed rule, stated as code: the runner may be registered on
    /// this host only if every required check passed. No checks at all, or
    /// any required check not passed, means not proven. An optional check
    /// failing is context, never proof.
    pub fn proves_runner(&self) -> bool {
        !self.checks.is_empty()
            && self
                .checks
                .iter()
                .all(|c| !c.required || c.status == CheckStatus::Passed)
    }

    /// The names of the required checks that did not pass — the shortest
    /// honest answer to "why is the runner absent?".
    pub fn failed_required(&self) -> Vec<&'static str> {
        self.checks
            .iter()
            .filter(|c| c.required && c.status == CheckStatus::Failed)
            .map(|c| c.name)
            .collect()
    }

    /// The full report as the spike's probe rendered it: verdict first, then
    /// every check with its evidence. Written where a human reads it; the
    /// verdict itself is consumed through [`Self::proves_runner`].
    pub fn render(&self) -> String {
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
            let required = if c.required { "required" } else { "optional" };
            let _ = writeln!(
                out,
                "\n[{}] {} check {}",
                c.status.label(),
                required,
                c.name
            );
            for line in c.detail.lines() {
                let _ = writeln!(out, "    {line}");
            }
        }
        out
    }
}
