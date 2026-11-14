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
fn defaults_are_reported_when_base_url_is_absent() {
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
    let lines = provider_lines(
        AiProvider::Anthropic,
        None,
        false,
        Some(4096),
        None,
        saya_config::CompactionMode::Auto,
    );
    assert!(lines.iter().any(|line| line.contains("unauthenticated")));
    let lines = provider_lines(
        AiProvider::Anthropic,
        None,
        true,
        Some(4096),
        None,
        saya_config::CompactionMode::Auto,
    );
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
    // The authority after the last `@` is what is reported; a password or
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
    // No port: the scheme's default port is used so the host is still reported.
    assert_eq!(
        parse_host_port("https://[2001:db8::1]"),
        Some(("2001:db8::1".into(), 443))
    );
}

#[test]
fn parse_host_port_rejects_strings_without_a_scheme() {
    // A bare hostname or host:port is not an endpoint; callers rely on None
    // here so doctor reports that no endpoint is configured.
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
fn provider_lines_report_an_ipv6_endpoint_without_an_explicit_port() {
    let lines = provider_lines(
        AiProvider::Ollama,
        Some("http://[::1]"),
        true,
        Some(4096),
        None,
        saya_config::CompactionMode::Auto,
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("configured endpoint: ::1:80")),
        "doctor should report the IPv6 endpoint on the http default port: {lines:?}"
    );
}

#[test]
fn closed_loopback_endpoint_is_reported_without_claiming_reachability() {
    let lines = provider_lines(
        AiProvider::Ollama,
        Some("http://127.0.0.1:1"),
        true,
        Some(4096),
        None,
        saya_config::CompactionMode::Auto,
    );
    let report = lines.join("\n");
    assert!(report.contains("configured endpoint: 127.0.0.1:1"));
    assert!(!report.contains("probe"));
    assert!(!report.contains("reachable"));
}

#[test]
fn provider_lines_names_the_stated_output_token_ceiling_without_a_default_note() {
    let lines = provider_lines(
        AiProvider::Ollama,
        None,
        true,
        Some(2048),
        None,
        saya_config::CompactionMode::Auto,
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "ai max_output_tokens: 2048"),
        "a stated ceiling is reported bare, with no default note: {lines:?}"
    );
}

#[test]
fn provider_lines_names_an_unset_output_token_ceiling_as_the_model_default() {
    let lines = provider_lines(
        AiProvider::Ollama,
        None,
        true,
        None,
        None,
        saya_config::CompactionMode::Auto,
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "ai max_output_tokens: unset (model default)"),
        "an unset ceiling is reported as the model's own default: {lines:?}"
    );
}

#[test]
fn provider_lines_name_the_configured_reasoning_effort_and_its_absence() {
    let configured = provider_lines(
        AiProvider::Ollama,
        None,
        true,
        None,
        Some(saya_config::ReasoningEffort::High),
        saya_config::CompactionMode::Auto,
    );
    assert!(
        configured
            .iter()
            .any(|line| line == "ai reasoning_effort: high"),
        "a stated effort is reported: {configured:?}"
    );
    let unset = provider_lines(
        AiProvider::Ollama,
        None,
        true,
        None,
        None,
        saya_config::CompactionMode::Auto,
    );
    assert!(
        unset
            .iter()
            .any(|line| line == "ai reasoning_effort: unset (adaptive)"),
        "an unset effort is reported as the adaptive policy: {unset:?}"
    );

    let adaptive = provider_lines(
        AiProvider::Ollama,
        None,
        true,
        None,
        Some(saya_config::ReasoningEffort::Adaptive),
        saya_config::CompactionMode::Auto,
    );
    assert!(
        adaptive
            .iter()
            .any(|line| line == "ai reasoning_effort: adaptive"),
        "an explicit adaptive value is reported: {adaptive:?}"
    );
}

#[test]
fn the_output_token_ceiling_line_is_informational_only() {
    for tokens in [Some(2048), None] {
        let lines = provider_lines(
            AiProvider::Ollama,
            None,
            true,
            tokens,
            None,
            saya_config::CompactionMode::Auto,
        );
        assert!(
            !lines.iter().any(|line| line.starts_with('!')),
            "the ceiling line must never warn: {lines:?}"
        );
    }
}

#[test]
fn the_ceiling_line_never_changes_whether_doctor_can_run() {
    // `can_run_query` only reads the profile and its secret state; the
    // ceiling line is informational, so a doctor that can run stays able
    // to run whatever the ceiling says. Asserted through `report`, the
    // same decision the exit code reads.
    for toml in [
        "[ai]\nmax_output_tokens = 2048\n",
        "[ai]\nmax_output_tokens = 4096\n",
        "",
    ] {
        let report = test_report(toml);
        assert!(
            report.can_run_query,
            "a resolvable profile must stay runnable (config {toml:?}): {:?}",
            report.lines
        );
        assert!(
            report
                .lines
                .iter()
                .any(|line| line.contains("max_output_tokens")),
            "the ceiling line is present in every case: {:?}",
            report.lines
        );
        assert_eq!(report.exit_code(), 0);
    }
}

/// Every resolved ceiling is reported by its dotted key, so `config doctor`
/// answers "what limits am I running under?" without a second command.
#[test]
fn doctor_reports_every_limits_key() {
    let report = test_report("");
    for key in [
        "limits.session.requests",
        "limits.session.tool_calls",
        "limits.session.elapsed_seconds",
        "limits.session.known_tokens",
        "limits.ask.requests",
        "limits.ask.tool_calls",
        "limits.ask.elapsed_seconds",
        "limits.ask.known_tokens",
        "limits.run.wall_clock_seconds",
        "limits.run.turns",
        "limits.run.tool_calls",
        "limits.run.tokens_per_endpoint",
        "limits.job.budget_mode",
        "limits.job.logical_requests",
        "limits.job.transport_attempts",
        "limits.job.tool_calls",
        "limits.job.known_tokens",
        "limits.job.job_tokens",
        "limits.job.elapsed_seconds",
    ] {
        assert!(
            report.lines.iter().any(|line| line.starts_with(key)),
            "config doctor does not report {key}: {:?}",
            report.lines
        );
    }
}

fn test_report(ai_toml: &str) -> DoctorReport {
    use crate::config::runtime::RuntimeConfig;
    use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve};
    let raw = format!(
        "default_profile = \"analytics\"\n{ai_toml}\n[profiles.analytics]\ntype = \"sqlite\"\npath = \"/tmp/x.sqlite\"\n"
    );
    let (config_raw, connections_raw) = raw.split_once("[profiles.").unwrap();
    let config = ConfigFile::from_toml(config_raw).unwrap();
    let connections = ConnectionsFile::from_toml(&format!("[profiles.{connections_raw}")).unwrap();
    let resolved = resolve(ResolutionInput::new(connections.clone()).with_user(config)).unwrap();
    report(&RuntimeConfig {
        resolved,
        connections,
        config_path: None,
        connections_path: None,
        cache_scope: std::path::PathBuf::from("/tmp/saya-doctor-test"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    })
}
