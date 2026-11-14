//! The startup fact set computed once at composition: whether the AI
//! provider is configured, phrased for the splash's state section. Facts
//! only — the details come from `config doctor --task ai`'s redacted
//! checks: no network, no database, and no secret value ever reaches the
//! screen.

use std::path::Path;

use crate::config::doctor_task::{CheckStatus, DoctorTask, readiness};
use crate::config::runtime::RuntimeConfig;

/// One need's readiness as the splash paints it: the verdict plus the
/// doctor detail that explains it — the provider check's detail when
/// ready ("ollama, model m"), the first failing check's when not.
#[derive(Debug)]
pub(crate) struct ReadinessLine {
    pub(crate) ready: bool,
    pub(crate) detail: String,
}

/// The startup fact set, computed once where the `App` is built — never
/// per frame.
#[derive(Debug)]
pub(crate) struct StartupReadiness {
    pub(crate) ai: ReadinessLine,
}

impl StartupReadiness {
    /// Reads the `ai` task's readiness from the resolved configuration
    /// only. The cwd is irrelevant to the task; `Path::new(".")` stands in.
    /// No network call, no secret value: the doctor module guarantees the
    /// details are redacted.
    pub(crate) fn from_runtime(runtime: &RuntimeConfig) -> Self {
        let report = readiness(runtime, DoctorTask::Ai, Path::new("."));
        let ai = if report.ready {
            // Ready: the provider check's own "ollama, model m" line is the
            // one fact worth naming.
            let detail = report
                .checks
                .iter()
                .find(|check| check.name == "provider")
                .map(|check| check.detail.clone())
                .unwrap_or_default();
            ReadinessLine {
                ready: true,
                detail,
            }
        } else {
            // Not ready: the first failing check says what is actually
            // missing — credentials, not the provider that already passed.
            let detail = report
                .checks
                .iter()
                .find(|check| check.status == CheckStatus::Fail)
                .map(|check| check.detail.clone())
                .unwrap_or_default();
            ReadinessLine {
                ready: false,
                detail,
            }
        };
        Self { ai }
    }
}

#[cfg(test)]
#[path = "startup_readiness_tests.rs"]
mod tests;
