//! `config doctor` — checks what a plain file listing cannot: do secrets
//! resolve, is the AI endpoint reachable, can profiles connect. Pure helpers
//! are separated so they are testable without a live environment.

use super::runtime::RuntimeConfig;
use saya_config::{AiProvider, MapSecretResolver, SecretResolver};
use saya_types::DatabaseProfile;

pub(crate) fn summary(runtime: &RuntimeConfig) -> String {
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
    lines.extend(secret_lines(runtime));
    lines.extend(provider_lines(
        runtime.resolved.ai.provider,
        runtime.resolved.ai.base_url.as_deref(),
        runtime.resolved.ai.api_key.is_some(),
    ));
    lines.join("\n")
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
fn secret_lines(runtime: &RuntimeConfig) -> Vec<String> {
    let resolver = MapSecretResolver::new(runtime.secret_values.clone());
    let mut lines = Vec::new();
    for (name, profile) in &runtime.connections.profiles {
        let references = profile_secrets(profile);
        if references.is_empty() {
            continue;
        }
        for label in references {
            match resolver.resolve(label) {
                Ok(_) => lines.push(format!("✓ {name}: {} resolves", label.redacted_label())),
                Err(error) => lines.push(format!(
                    "✗ {name}: {} does not resolve ({error})",
                    label.redacted_label()
                )),
            }
        }
    }
    if lines.is_empty() {
        lines.push("secrets: none referenced".to_string());
    }
    lines
}

fn profile_secrets(profile: &DatabaseProfile) -> Vec<&saya_types::SecretRef> {
    match profile {
        DatabaseProfile::Postgres { password, .. } => password.iter().collect(),
        DatabaseProfile::Mysql {
            password, ssl_ca, ..
        } => password.iter().chain(ssl_ca.iter()).collect(),
        DatabaseProfile::DuckDb { .. } | DatabaseProfile::Sqlite { .. } => Vec::new(),
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

/// Minimal `scheme://host[:port]` extraction without pulling in a URL crate.
/// A string without a scheme is not treated as a host.
fn parse_host_port(url: &str) -> Option<(String, u16)> {
    let scheme_end = url.find("://")?;
    let is_tls = url[..scheme_end].eq_ignore_ascii_case("https");
    let after_scheme = &url[scheme_end + 3..];
    let authority = after_scheme.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    if let Some((host, port)) = authority.rsplit_once(':') {
        let trimmed = host.trim_matches(|character| character == '[' || character == ']');
        return Some((trimmed.to_string(), port.parse().ok()?));
    }
    Some((authority.to_string(), if is_tls { 443 } else { 80 }))
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
