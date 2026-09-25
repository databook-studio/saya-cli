//! The run directory's JSON records: the spec (`spec.json`) a resume
//! reloads, and the bound plan (`plan.json`) a resume replays.
//!
//! The store is metadata only — no goal or plan text reaches it — so these
//! files are where the approval's full shape lives. They are written 0600:
//! the run directory is already 0700, but these carry the goal and scopes,
//! so they stay owner-only too. A spec that cannot be persisted fails the
//! run before it starts: a run that cannot state its spec on disk cannot be
//! resumed, so it does not start.

use saya_types::{RunPlan, RunSpec};
use std::path::{Path, PathBuf};

const SPEC_FILE: &str = "spec.json";
const PLAN_FILE: &str = "plan.json";

fn write_private(path: &Path, json: &str) -> std::io::Result<()> {
    std::fs::write(path, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Persists the run spec. A failure here fails the run before it starts: a
/// run that cannot state its spec on disk cannot be resumed, so it does not
/// start.
pub(super) fn persist_spec(dir: &Path, spec: &RunSpec) -> std::io::Result<()> {
    let json = serde_json::to_string(spec).map_err(std::io::Error::other)?;
    write_private(&dir.join(SPEC_FILE), &json)
}

/// Persists the bound plan — the layered one, with step budgets inherited
/// where the plan left them unset.
pub(super) fn persist_plan(dir: &Path, plan: &RunPlan) -> std::io::Result<()> {
    let json = serde_json::to_string(plan).map_err(std::io::Error::other)?;
    write_private(&dir.join(PLAN_FILE), &json)
}

/// Loads a run's spec; the error text names the missing file cleanly.
pub(super) fn load_spec(dir: &Path) -> Result<RunSpec, String> {
    let path = dir.join(SPEC_FILE);
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| format!("run spec ({SPEC_FILE}) could not be read: {error}"))?;
    serde_json::from_str(&raw).map_err(|error| format!("run spec is unreadable: {error}"))
}

/// Loads a run's bound plan.
pub(super) fn load_plan(dir: &Path) -> Result<RunPlan, String> {
    let path: PathBuf = dir.join(PLAN_FILE);
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| format!("run plan ({PLAN_FILE}) could not be read: {error}"))?;
    serde_json::from_str(&raw).map_err(|error| format!("run plan is unreadable: {error}"))
}
