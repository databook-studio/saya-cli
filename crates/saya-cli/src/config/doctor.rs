//! `config doctor` — checks what a plain file listing cannot: do secrets
//! resolve, what AI endpoint is configured, and is the selected profile
//! configured enough to try. It performs no network checks. Pure helpers are
//! separated so they are testable without a live environment.

use super::runtime::RuntimeConfig;

mod details;

#[cfg(test)]
use details::parse_host_port;
pub(crate) use details::{profile_secrets, provider_endpoint};
use details::{provider_lines, secret_lines};
#[cfg(test)]
use saya_config::AiProvider;

#[cfg(test)]
#[path = "doctor/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "doctor/ignored_override_tests.rs"]
mod ignored_override_tests;

/// A doctor diagnosis: the report lines and whether the setup can plausibly run
/// a query. `can_run_query` is false when nothing is configured or the selected
/// profile cannot connect (an unresolved secret); warnings stay true here, so a
/// missing cloud API key or an ignored project override does not by itself make
/// doctor fail.
pub(crate) struct DoctorReport {
    pub(crate) lines: Vec<String>,
    pub(crate) can_run_query: bool,
}

impl DoctorReport {
    /// 0 when the setup can run a query, 3 (connection/config) when it cannot.
    pub(crate) fn exit_code(&self) -> i32 {
        if self.can_run_query { 0 } else { 3 }
    }
}

pub(crate) fn report(runtime: &RuntimeConfig) -> DoctorReport {
    let mut lines = vec![
        format!("config file: {}", path(&runtime.config_path)),
        format!("connections file: {}", path(&runtime.connections_path)),
        format!("profiles: {}", runtime.connections.profiles.len()),
        format!(
            "selected profile: {}",
            runtime.resolved.profile_name.as_deref().unwrap_or("none")
        ),
    ];
    lines.extend(ignored_override_lines(
        &runtime.resolved.ignored_project_overrides,
    ));
    let secret_report = secret_lines(runtime);
    let selected_unresolved = secret_report.selected_unresolved;
    lines.extend(secret_report.lines);
    lines.extend(provider_lines(
        runtime.resolved.ai.provider,
        runtime.resolved.ai.base_url.as_deref(),
        runtime.resolved.ai.api_key.is_some(),
        runtime.resolved.ai.max_output_tokens,
        runtime.resolved.ai.max_output_tokens_is_default,
        runtime.resolved.ai.compaction,
    ));
    lines.extend(advice_lines(runtime, selected_unresolved));
    DoctorReport {
        lines,
        can_run_query: can_run_query(runtime, selected_unresolved),
    }
}

/// The TUI's `/doctor` shows the report text; it does not consume the exit code.
pub(crate) fn summary(runtime: &RuntimeConfig) -> String {
    report(runtime).lines.join("\n")
}

/// Actionable next steps. The factual lines above stay; this only adds. A first
/// run that "ends somewhere" needs doctor to say what to do, not just that
/// something is missing.
fn advice_lines(runtime: &RuntimeConfig, selected_unresolved: bool) -> Vec<String> {
    let mut advice: Vec<String> = Vec::new();
    if runtime.config_path.is_none() && runtime.connections_path.is_none() {
        advice.push(
            "  nothing is configured — run `saya config init` to create starter \
             config in your user directory."
                .into(),
        );
    } else if runtime.config_path.is_none() {
        advice.push(
            "  no config.toml found — run `saya config init` to write one to your \
             user directory."
                .into(),
        );
    } else if runtime.connections_path.is_none() {
        advice.push(
            "  no connections.toml found — run `saya config init` to add a starter \
             profile."
                .into(),
        );
    }
    if runtime.resolved.profile.is_none()
        && !advice.iter().any(|line| line.contains("saya config init"))
    {
        advice.push(
            "  no profile is selected — run `saya config init`, or set \
             `default_profile` in your config."
                .into(),
        );
    }
    if selected_unresolved {
        advice.push(
            "  the selected profile's secret does not resolve — set the referenced \
             environment variable (or add it to a .env.saya file passed with \
             --env-file), then re-run."
                .into(),
        );
    }
    advice
}

fn can_run_query(runtime: &RuntimeConfig, selected_unresolved: bool) -> bool {
    runtime.resolved.profile.is_some() && !selected_unresolved
}

/// Names the security-critical settings the project layer tried to change and
/// did not get. The one-shot warning is deliberately short, so this is where a
/// user finds out *which* settings were ignored and what to do about it —
/// doctor is the "what is wrong with my setup" command, and this is its
/// subject. Empty when the project layer changed none of them.
fn ignored_override_lines(ignored: &[String]) -> Vec<String> {
    if ignored.is_empty() {
        return Vec::new();
    }
    vec![
        format!(
            "ignored from project config: {} (the project layer is untrusted)",
            ignored.join(", ")
        ),
        "  these decide where your API key is sent, whether rows leave the machine,".into(),
        "  and whether read-only enforcement stays on — so a cloned repository does".into(),
        "  not get to set them. Move them to your user config to have them applied,".into(),
        "  or pass --trust-project-config to accept this project's values.".into(),
    ]
}

fn path(value: &Option<std::path::PathBuf>) -> String {
    value
        .as_deref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "not found".into())
}
