//! `config doctor --task <query|ai|coding>` — one task's readiness, computed
//! from the resolved configuration only. No network call, no database
//! connection, no provider call, and no secret value ever leaves this module:
//! details name redacted reference labels, never values.

mod ai;

use std::path::Path;

use saya_config::{MapSecretResolver, SecretResolver};

use super::runtime::RuntimeConfig;
use crate::interactive::session_workspace;

/// The task whose readiness `config doctor --task` reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DoctorTask {
    Query,
    Ai,
    Coding,
}

impl DoctorTask {
    /// The stable task name on the wire and in text output.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Ai => "ai",
            Self::Coding => "coding",
        }
    }
}

/// One check's verdict, serialized snake_case (`ok|warn|fail`).
///
/// `pub` + a crate-root re-export (see lib.rs): a `TerminalEvent::DoctorTask`
/// variant carries `Vec<TaskCheck>`, and a public enum may not expose a
/// crate-only type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

/// One named readiness check. `name` is stable API; `detail` is redacted text.
/// A failing or warning check may carry a `hint` naming the setting or file
/// to edit — never a secret value; the wire omits the key when there is none.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TaskCheck {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// One task's readiness: `{"task":…,"ready":…,"checks":[…]}` on the wire.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct TaskReadiness {
    pub(crate) task: &'static str,
    pub(crate) ready: bool,
    pub(crate) checks: Vec<TaskCheck>,
}

pub(crate) fn readiness(runtime: &RuntimeConfig, task: DoctorTask, cwd: &Path) -> TaskReadiness {
    match task {
        DoctorTask::Query => query_readiness(runtime),
        DoctorTask::Ai => ai::readiness(runtime),
        DoctorTask::Coding => coding_readiness(runtime, cwd),
    }
}

fn check(name: &str, status: CheckStatus, detail: impl Into<String>) -> TaskCheck {
    TaskCheck {
        name: name.to_string(),
        status,
        detail: detail.into(),
        hint: None,
    }
}

/// A check with an edit hint: only failing or warning checks carry one, and
/// the text names settings, files, or reference labels — never a value.
fn check_hint(
    name: &str,
    status: CheckStatus,
    detail: impl Into<String>,
    hint: impl Into<String>,
) -> TaskCheck {
    TaskCheck {
        name: name.to_string(),
        status,
        detail: detail.into(),
        hint: Some(hint.into()),
    }
}

/// The user config file path: the layer a user edits, and the one every
/// edit hint points at.
pub(crate) fn user_config_file() -> String {
    super::sources::user_config_dir()
        .join("config.toml")
        .display()
        .to_string()
}

fn ready(task: &'static str, checks: Vec<TaskCheck>) -> TaskReadiness {
    let ready = checks.iter().all(|check| check.status != CheckStatus::Fail);
    TaskReadiness {
        task,
        ready,
        checks,
    }
}

const NO_PROFILE_SELECTED: &str = "no connection profile is selected — add one to connections.toml or pass \
     --connections/--profile";

/// The edit hint for a query task with no selected profile.
const NO_PROFILE_HINT: &str = "Add a profile to connections.toml or pass \
     --connections/--profile; `saya setup` can do it.";

/// Ready iff a profile is selected and every referenced secret resolves —
/// exactly `can_run_query`, so the per-task exit code and the flat report
/// cannot disagree.
fn query_readiness(runtime: &RuntimeConfig) -> TaskReadiness {
    let profile = match runtime.resolved.profile.as_ref() {
        Some(profile) => check(
            "profile",
            CheckStatus::Ok,
            format!(
                "selected: {} ({})",
                runtime.resolved.profile_name.as_deref().unwrap_or("?"),
                profile.dialect().as_str()
            ),
        ),
        None => check_hint(
            "profile",
            CheckStatus::Fail,
            NO_PROFILE_SELECTED,
            NO_PROFILE_HINT,
        ),
    };
    ready("query", vec![profile, secrets_check(runtime)])
}

const SECRETS_OK: &str = "every referenced secret resolves";

/// The edit hint when a profile's secrets do not resolve: the check's detail
/// already names the redacted labels, so the hint only says what to do with
/// them — values are never printed.
const SECRETS_HINT: &str = "Export the variables named above; values are never printed.";

fn secrets_check(runtime: &RuntimeConfig) -> TaskCheck {
    let Some(profile) = runtime.resolved.profile.as_ref() else {
        return check("secrets", CheckStatus::Ok, SECRETS_OK);
    };
    let resolver = MapSecretResolver::new(runtime.secret_values.clone());
    let failures: Vec<String> = super::doctor::profile_secrets(profile)
        .into_iter()
        .filter_map(|reference| {
            resolver
                .resolve(reference)
                .err()
                .map(|error| format!("{} does not resolve ({error})", reference.redacted_label()))
        })
        .collect();
    match failures.is_empty() {
        true => check("secrets", CheckStatus::Ok, SECRETS_OK),
        false => check_hint(
            "secrets",
            CheckStatus::Fail,
            failures.join("; "),
            SECRETS_HINT,
        ),
    }
}

/// The `ai` checks plus `workspace`. The database is NOT consulted: a
/// coding-ready config with zero profiles is ready.
fn coding_readiness(runtime: &RuntimeConfig, cwd: &Path) -> TaskReadiness {
    const WORKSPACE_HINT: &str = "Launch inside a git worktree or pass \
         --workspace <dir> (not available to config doctor: run saya from the project).";
    let mut checks = ai::readiness(runtime).checks;
    checks.push(match session_workspace::resolve_root(None, cwd) {
        Ok(Some(root)) => check("workspace", CheckStatus::Ok, root.display().to_string()),
        Ok(None) => check_hint(
            "workspace",
            CheckStatus::Fail,
            "no project directory here — run inside a git worktree",
            WORKSPACE_HINT,
        ),
        Err(error) => check_hint("workspace", CheckStatus::Fail, error, WORKSPACE_HINT),
    });
    if runtime.resolved.coding.enabled {
        checks.push(check_hint("native_runtime",CheckStatus::Warn,"native coding is enabled; configuration inspection does not prove provider tools or host confinement","Run saya coding doctor --probe from the project for actual native readiness checks."));
    }
    ready("coding", checks)
}

#[cfg(test)]
#[path = "doctor_task_tests.rs"]
mod tests;
