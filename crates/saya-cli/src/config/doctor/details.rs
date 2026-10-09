use super::super::runtime::RuntimeConfig;
use saya_config::{AiProvider, MapSecretResolver, SecretResolver};
use saya_types::DatabaseProfile;
use url::Url;

pub(super) struct SecretReport {
    pub(super) lines: Vec<String>,
    /// True when the *selected* profile has a referenced secret that does not
    /// resolve — the one case that means a query cannot connect.
    pub(super) selected_unresolved: bool,
}

pub(super) fn secret_lines(runtime: &RuntimeConfig) -> SecretReport {
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

/// Dry-runs every secret reference in every profile through the resolver.
/// Values are never printed — only whether each reference resolves.
pub(crate) fn profile_secrets(profile: &DatabaseProfile) -> Vec<&saya_types::SecretRef> {
    match profile {
        DatabaseProfile::Postgres { password, .. } => password.iter().collect(),
        DatabaseProfile::Mysql {
            password, ssl_ca, ..
        } => password.iter().chain(ssl_ca.iter()).collect(),
        DatabaseProfile::DuckDb { .. } | DatabaseProfile::Sqlite { .. } => Vec::new(),
        DatabaseProfile::ClickHouse { password, .. } => password.iter().collect(),
        DatabaseProfile::BigQuery {
            service_account_key,
            ..
        } => std::iter::once(service_account_key).collect(),
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

/// Host/port the AI provider is configured to contact. `None` means there is
/// no meaningful endpoint to report (a gateway with no default address).
pub(crate) fn provider_endpoint(
    provider: AiProvider,
    base_url: Option<&str>,
) -> Option<(String, u16)> {
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
/// and an empty authority has no host to report. The port defaults from the
/// scheme (`https` => 443, `http` => 80); schemes with no known default are not
/// reported. IPv6 hosts are returned without their brackets, matching the
/// display shape of the configured-endpoint line.
pub(super) fn parse_host_port(url: &str) -> Option<(String, u16)> {
    // `Url::parse` is lenient about an empty authority: for a special scheme it
    // folds a leading path segment into the host ("http:///path" => host
    // "path"), which would have doctor treat a string that is not a host as one. A URL
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
pub(super) fn provider_lines(
    provider: AiProvider,
    base_url: Option<&str>,
    has_key_ref: bool,
    max_output_tokens: u32,
    max_output_tokens_is_default: bool,
    compaction: saya_config::CompactionMode,
) -> Vec<String> {
    let mut lines = vec![format!(
        "ai provider: {} model: (from config)",
        provider.as_str()
    )];
    if max_output_tokens_is_default {
        lines.push(format!(
            "ai max_output_tokens: {max_output_tokens} (default 4096 when unset)"
        ));
    } else {
        lines.push(format!("ai max_output_tokens: {max_output_tokens}"));
    }
    lines.push(format!("ai compaction: {}", compaction.as_str()));
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
        Some((host, port)) => lines.push(format!("configured endpoint: {host}:{port}")),
        None => lines.push("configured endpoint: none".to_string()),
    }
    lines
}
