//! `config doctor` — checks what a plain file listing cannot: do secrets
//! resolve, is the AI endpoint reachable, can profiles connect. Pure helpers
//! are separated so they are testable without a live environment.

use super::runtime::RuntimeConfig;
use saya_config::{AiProvider, MapSecretResolver, SecretResolver};
use saya_types::DatabaseProfile;
use url::Url;

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

/// Dry-runs every secret reference in every profile through the resolver.
/// Values are never printed — only whether each reference resolves.
struct SecretReport {
    lines: Vec<String>,
    /// True when the *selected* profile has a referenced secret that does not
    /// resolve — the one case that means a query cannot connect.
    selected_unresolved: bool,
}

fn secret_lines(runtime: &RuntimeConfig) -> SecretReport {
    let resolver = MapSecretResolver::new(runtime.secret_values.clone());
    let selected = runtime.resolved.profile_name.as_deref();
    let mut lines = Vec::new();
    let mut selected_unresolved = false;
    for (name, profile) in &runtime.connections.profiles {
        let references = profile_secrets(profile);
        if references.is_empty() {
            continue;
        }
        for label in references {
            match resolver.resolve(label) {
                Ok(_) => lines.push(format!("✓ {name}: {} resolves", label.redacted_label())),
                Err(error) => {
                    if Some(name.as_str()) == selected {
                        selected_unresolved = true;
                    }
                    lines.push(format!(
                        "✗ {name}: {} does not resolve ({error})",
                        label.redacted_label()
                    ));
                }
            }
        }
    }
    if lines.is_empty() {
        lines.push("secrets: none referenced".to_string());
    }
    SecretReport {
        lines,
        selected_unresolved,
    }
}

fn profile_secrets(profile: &DatabaseProfile) -> Vec<&saya_types::SecretRef> {
    match profile {
        DatabaseProfile::Postgres { password, .. } => password.iter().collect(),
        DatabaseProfile::Mysql {
            password, ssl_ca, ..
        } => password.iter().chain(ssl_ca.iter()).collect(),
        DatabaseProfile::DuckDb { .. } | DatabaseProfile::Sqlite { .. } => Vec::new(),
        DatabaseProfile::ClickHouse { password, .. } => password.iter().collect(),
        DatabaseProfile::Snowflake {
            private_key,
            password,
            passphrase,
            ..
        } => private_key
            .iter()
            .chain(password.iter())
            .chain(passphrase.iter())
            .collect(),
    }
}

/// Host/port the AI provider would actually be contacted on, so doctor can
/// probe it. `None` means there is nothing meaningful to probe (a gateway
/// with no default address).
fn provider_endpoint(provider: AiProvider, base_url: Option<&str>) -> Option<(String, u16)> {
    const DEFAULTS: [(AiProvider, &str, u16); 4] = [
        (AiProvider::Ollama, "localhost", 11_434),
        (AiProvider::Openai, "api.openai.com", 443),
        (AiProvider::Anthropic, "api.anthropic.com", 443),
        (AiProvider::Gemini, "generativelanguage.googleapis.com", 443),
    ];
    if let Some(url) = base_url {
        return parse_host_port(url);
    }
    DEFAULTS
        .iter()
        .find(|(candidate, _, _)| *candidate == provider)
        .map(|(_, host, port)| ((*host).to_string(), *port))
}

/// Host/port the AI provider would be contacted on, parsed from `base_url`
/// with `url::Url`. Returns `None` when there is no usable endpoint: a string
/// without a scheme is not an endpoint, a `mailto:`-style URL carries no host,
/// and an empty authority has no host to probe. The port defaults from the
/// scheme (`https` => 443, `http` => 80); schemes with no known default are not
/// probed. IPv6 hosts are returned without their brackets, matching the display
/// shape of the `probe:` line.
fn parse_host_port(url: &str) -> Option<(String, u16)> {
    // `Url::parse` is lenient about an empty authority: for a special scheme it
    // folds a leading path segment into the host ("http:///path" => host
    // "path"), which would have doctor probe a string that is not a host. A URL
    // whose authority is empty is therefore rejected from the raw input before
    // parsing, so `Url::parse` is only reached when there is a host to extract.
    let scheme_end = url.find("://")?;
    let authority_start = scheme_end + 3;
    let first = url.as_bytes().get(authority_start).copied()?;
    if matches!(first, b'/' | b'?' | b'#') {
        return None;
    }
    let parsed = Url::parse(url).ok()?;
    let host = parsed.host_str()?.trim_matches(|c| c == '[' || c == ']');
    let port = parsed.port_or_known_default()?;
    Some((host.to_string(), port))
}

/// What doctor says about the AI side before any network I/O.
fn provider_lines(provider: AiProvider, base_url: Option<&str>, has_key_ref: bool) -> Vec<String> {
    let mut lines = vec![format!(
        "ai provider: {} model: (from config)",
        provider.as_str()
    )];
    if matches!(
        provider,
        AiProvider::Openai | AiProvider::Anthropic | AiProvider::Gemini
    ) && !has_key_ref
    {
        lines.push(format!(
            "! no api_key reference configured — requests to {} will be unauthenticated",
            provider.as_str()
        ));
    }
    match provider_endpoint(provider, base_url) {
        Some((host, port)) => lines.push(format!("probe: {host}:{port}")),
        None => lines.push("probe: no endpoint to check".to_string()),
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_and_port_extraction_handles_common_shapes() {
        assert_eq!(
            parse_host_port("http://localhost:11434"),
            Some(("localhost".into(), 11_434))
        );
        assert_eq!(
            parse_host_port("https://api.anthropic.com/v1"),
            Some(("api.anthropic.com".into(), 443))
        );
        assert_eq!(
            parse_host_port("http://10.0.0.4:8080/v1"),
            Some(("10.0.0.4".into(), 8080))
        );
        assert_eq!(parse_host_port("not a url"), None);
    }

    #[test]
    fn defaults_are_probed_when_base_url_is_absent() {
        assert_eq!(
            provider_endpoint(AiProvider::Ollama, None),
            Some(("localhost".into(), 11_434))
        );
        // A configured base_url wins over the default.
        assert_eq!(
            provider_endpoint(AiProvider::Ollama, Some("http://box.lan:9999")),
            Some(("box.lan".into(), 9_999))
        );
    }

    #[test]
    fn cloud_provider_without_key_reference_warns() {
        let lines = provider_lines(AiProvider::Anthropic, None, false);
        assert!(lines.iter().any(|line| line.contains("unauthenticated")));
        let lines = provider_lines(AiProvider::Anthropic, None, true);
        assert!(!lines.iter().any(|line| line.contains("unauthenticated")));
    }

    #[test]
    fn parse_host_port_extracts_a_plain_https_host_and_port() {
        assert_eq!(
            parse_host_port("https://api.openai.com:5432"),
            Some(("api.openai.com".into(), 5432))
        );
    }

    #[test]
    fn parse_host_port_strips_userinfo_before_host_and_port() {
        // The authority after the last `@` is what is probed; a password or
        // username never reaches the host/port split.
        assert_eq!(
            parse_host_port("https://user:pass@host:5432"),
            Some(("host".into(), 5432))
        );
        // Userinfo in front of an IPv6 literal with no explicit port still
        // resolves to the host and the scheme's default port.
        assert_eq!(
            parse_host_port("https://user:pass@[::1]"),
            Some(("::1".into(), 443))
        );
    }

    #[test]
    fn parse_host_port_handles_ipv6_literals() {
        assert_eq!(
            parse_host_port("http://[::1]:5432"),
            Some(("::1".into(), 5432))
        );
        // No port: the scheme's default port is used so the host is still probed.
        assert_eq!(
            parse_host_port("https://[2001:db8::1]"),
            Some(("2001:db8::1".into(), 443))
        );
    }

    #[test]
    fn parse_host_port_rejects_strings_without_a_scheme() {
        // A bare hostname or host:port is not an endpoint; callers rely on None
        // here so doctor reports "no endpoint to check" instead of probing one.
        assert_eq!(parse_host_port("api.openai.com"), None);
        assert_eq!(parse_host_port("localhost:11434"), None);
        assert_eq!(parse_host_port("not a url"), None);
    }

    #[test]
    fn parse_host_port_rejects_an_empty_authority() {
        // `http:///path` has no host; probing it would dial an empty address.
        assert_eq!(parse_host_port("http:///path"), None);
    }

    #[test]
    fn provider_lines_probe_an_ipv6_endpoint_without_an_explicit_port() {
        let lines = provider_lines(AiProvider::Ollama, Some("http://[::1]"), true);
        assert!(
            lines.iter().any(|line| line.contains("probe: ::1:80")),
            "doctor should probe the IPv6 endpoint on the http default port: {lines:?}"
        );
    }
}

#[cfg(test)]
mod ignored_override_tests {
    use super::ignored_override_lines;

    /// The one-shot warning is deliberately terse, so doctor is the only place
    /// a user can learn *which* settings were ignored. If this stops reporting
    /// them, the terse warning becomes a dead end.
    #[test]
    fn doctor_names_every_ignored_setting_and_the_way_to_apply_it() {
        let report =
            ignored_override_lines(&["ai.base_url".to_string(), "run.read_only".to_string()])
                .join("\n");

        assert!(
            report.contains("ai.base_url") && report.contains("run.read_only"),
            "doctor must name each ignored setting: {report}"
        );
        // Both routes: the trusted one (move them) and the override.
        assert!(
            report.contains("user config") && report.contains("--trust-project-config"),
            "doctor must say how to apply them: {report}"
        );
    }

    /// A project layer that changed nothing adds no noise to the report.
    #[test]
    fn doctor_is_silent_when_nothing_was_ignored() {
        assert!(
            ignored_override_lines(&[]).is_empty(),
            "no ignored settings means no section"
        );
    }
}
