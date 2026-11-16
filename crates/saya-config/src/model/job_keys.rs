//! The `[job]` / `[jobs]` collision, diagnosed.
//!
//! The two tables sit one letter apart and mean different things: `[job]` is
//! the job host's own settings, `[jobs]` is a run's download budgets. Both
//! types deny unknown fields, so a key written in the wrong one of the two
//! would otherwise fail with serde's `unknown field`, which cannot say which
//! table was meant. The keys are read from the raw text here, the way a retired
//! key is, and each is refused naming the table it belongs to.

use crate::ConfigError;

/// The `[jobs]` keys that are not `[job]` settings, each with the fix: the
/// table's one live setting, and the four run ceilings that live in
/// `[limits.run]`. Named here rather than derived, because the guard
/// must recognise a key `[jobs]` itself does not accept — the type cannot
/// supply that.
const JOBS_KEYS: &[(&str, &str)] = &[
    ("fetch", "write [jobs.fetch] instead"),
    (
        "wall_clock_seconds",
        "write limits.run.wall_clock_seconds instead",
    ),
    (
        "tokens_per_endpoint",
        "write limits.run.tokens_per_endpoint instead",
    ),
    ("turns", "write limits.run.turns instead"),
    ("tool_calls", "write limits.run.tool_calls instead"),
];

/// The `[job]` keys `[jobs]` must refuse by name.
const JOB_KEYS: &[&str] = &[
    "automatic_lsp_profile",
    "extensions",
    "program_dir",
    "root_mode",
];

/// Names the first key written in the wrong one of the two tables, if any.
pub(super) fn misplaced_job_key(raw: &str) -> Option<ConfigError> {
    let value: toml::Value = toml::from_str(raw).ok()?;
    let table = value.as_table()?;
    if let Some(job) = table.get("job").and_then(toml::Value::as_table) {
        for key in job.keys() {
            if let Some((key, fix)) = JOBS_KEYS.iter().find(|(name, _)| *name == key.as_str()) {
                return Some(ConfigError::MisplacedJobKey {
                    message: format!(
                        "[job] {key} is not a job-host setting: [jobs] is a run's budgets, \
                         not the job host's — {fix}"
                    ),
                });
            }
        }
    }
    if let Some(jobs) = table.get("jobs").and_then(toml::Value::as_table) {
        for key in jobs.keys() {
            if JOB_KEYS.contains(&key.as_str()) {
                return Some(ConfigError::MisplacedJobKey {
                    message: format!(
                        "[jobs] {key} is not a run budget: [job] is the job host's own \
                         settings — write job.{key} instead"
                    ),
                });
            }
        }
    }
    None
}
