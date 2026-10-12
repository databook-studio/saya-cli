use super::{CodingApplication, CodingError};
use serde_json::{Value, json};

impl CodingApplication {
    /// Guided readiness: the doctor's facts as an ordered checklist, each
    /// step either `ok` or `blocked` with the one action that clears it.
    /// Nothing is installed, enabled or written; the report carries no
    /// credential, endpoint or provider binding.
    pub async fn setup(&self, probe_provider: bool) -> Result<Value, CodingError> {
        Ok(checklist(&self.doctor(probe_provider).await?))
    }
}

fn step(check: &str, ok: bool, action: &str) -> Value {
    json!({"check": check, "status": if ok { "ok" } else { "blocked" }, "action": if ok { Value::Null } else { json!(action) }})
}

pub(super) fn checklist(report: &Value) -> Value {
    let flag = |key: &str| report.get(key).and_then(Value::as_bool) == Some(true);
    let probed = report
        .get("native_provider_tool_loop")
        .and_then(Value::as_bool);
    let steps = vec![
        step(
            "coding_enabled",
            flag("enabled"),
            "Set coding.enabled = true in your trusted user configuration (project configuration cannot enable it)",
        ),
        step(
            "git",
            report.get("git").is_some_and(Value::is_string),
            "Install Git and put it on PATH",
        ),
        step(
            "private_state",
            flag("private_state_writable"),
            "Make the saya state directory private and writable by you, outside the repository",
        ),
        step(
            "command_confinement",
            flag("confinement_proven"),
            "This host could not prove command confinement; coding commands stay unavailable here",
        ),
        step(
            "check_programs",
            flag("check_programs_installed"),
            "Install the programs your configured checks name in the trusted program directory (nothing is installed for you)",
        ),
        step(
            "model_tool_loop",
            probed == Some(true),
            if probed == Some(false) {
                "The configured model did not complete the native tool loop: choose a model that supports tool calls"
            } else {
                "Run `coding setup --probe` to let saya make one bounded call to the configured model"
            },
        ),
    ];
    let ready = flag("ready");
    json!({
        "ready": ready,
        "steps": steps,
        "blocked": report.get("blocked").cloned().unwrap_or_else(|| json!([])),
        "installs_nothing": true
    })
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod setup_tests;
