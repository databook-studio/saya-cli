use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Status {
    Passed,
    Failed,
}

pub(super) struct Check {
    pub(super) name: &'static str,
    pub(super) required: bool,
    pub(super) status: Status,
    pub(super) detail: String,
}

impl Check {
    pub(super) fn passed(name: &'static str, required: bool, detail: impl Into<String>) -> Self {
        Self {
            name,
            required,
            status: Status::Passed,
            detail: detail.into(),
        }
    }

    pub(super) fn failed(name: &'static str, required: bool, detail: impl Into<String>) -> Self {
        Self {
            name,
            required,
            status: Status::Failed,
            detail: detail.into(),
        }
    }

    pub(super) fn info(name: &'static str, detail: impl Into<String>) -> Self {
        Self::passed(name, false, detail)
    }
}

pub(super) struct ProbeReport {
    pub(super) platform: &'static str,
    pub(super) kernel: String,
    pub(super) checks: Vec<Check>,
}

impl ProbeReport {
    /// The fail-closed rule, stated as code: the runner may be registered on
    /// this host only if every required check passed. No checks at all, or any
    /// required check not passed, means not proven. An optional check failing
    /// is context, never proof.
    pub(super) fn proves_runner(&self) -> bool {
        !self.checks.is_empty()
            && self
                .checks
                .iter()
                .all(|c| !c.required || c.status == Status::Passed)
    }

    pub(super) fn render(&self) -> String {
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

pub(super) fn indent(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let _ = writeln!(out, "    {line}");
    }
    out
}
