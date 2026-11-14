use super::{CodingApplication, CodingError};
use serde_json::{Value, json};

impl CodingApplication {
    /// Guided readiness: the doctor's facts as an ordered checklist, each
    /// step `ok`, `optional` (not configured, and not required), or `blocked`
    /// with the one action that clears it. Only a blocked step makes the
    /// install unready. Nothing is installed, enabled or written; the report
    /// carries no credential, endpoint or provider binding.
    pub async fn setup(&self, probe_provider: bool) -> Result<Value, CodingError> {
        Ok(checklist(&self.doctor(probe_provider).await?))
    }
}

fn step(check: &str, status: &str, action: &str) -> Value {
    json!({"check": check, "status": status, "action": if status == "ok" { Value::Null } else { json!(action) }})
}

pub(super) fn checklist(report: &Value) -> Value {
    let flag = |key: &str| report.get(key).and_then(Value::as_bool) == Some(true);
    let probed = report
        .get("native_provider_tool_loop")
        .and_then(Value::as_bool);
    let steps = vec![
        step(
            "git",
            if report.get("git").is_some_and(Value::is_string) {
                "ok"
            } else {
                "blocked"
            },
            "Install Git and put it on PATH",
        ),
        step(
            "private_state",
            if flag("private_state_writable") {
                "ok"
            } else {
                "blocked"
            },
            "Make the saya state directory private and writable by you, outside the repository",
        ),
        step(
            "command_confinement",
            if flag("confinement_proven") {
                "ok"
            } else if cfg!(target_os = "macos") {
                "blocked"
            } else {
                // The Linux probe fails closed by design; confinement is a
                // macOS capability, not a missing Linux one.
                "optional"
            },
            if cfg!(target_os = "macos") {
                "This host could not prove command confinement; job commands stay unavailable here"
            } else {
                "Host command confinement is not available on this platform; job commands run in the host lane"
            },
        ),
        step(
            "model_tool_loop",
            match probed {
                Some(true) => "ok",
                Some(false) => "blocked",
                // An unrun probe is an optional check, not a missing
                // capability: a fresh install with a configured provider is
                // ready until the probe actually fails.
                None => "optional",
            },
            if probed == Some(false) {
                "The configured model did not complete the native tool loop: choose a model that supports tool calls"
            } else {
                "Run `saya job setup --probe` to let saya make one bounded call to the configured model"
            },
        ),
    ];
    let ready = flag("ready");
    let mut checklist = json!({
        "ready": ready,
        "steps": steps,
        "blocked": report.get("blocked").cloned().unwrap_or_else(|| json!([])),
        "optional": report.get("optional").cloned().unwrap_or_else(|| json!([])),
        "installs_nothing": true
    });
    // An in-place root has no worktree: the checklist names the folder it
    // will edit, so readiness never implies a checkout that does not exist.
    if report.get("mode").and_then(Value::as_str) == Some("in_place") {
        checklist["mode"] = json!("in_place");
        checklist["root"] = report.get("root").cloned().unwrap_or(Value::Null);
    }
    checklist
}

#[cfg(all(test, unix))]
#[path = "setup_tests.rs"]
mod setup_tests;
