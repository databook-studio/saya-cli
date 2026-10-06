//! Unit tests for `config doctor --task`: per-task readiness computed from
//! the resolved configuration only — no network, no database connection, and
//! no secret value ever serialized. The runtime fixtures hand-build a
//! `RuntimeConfig` the same way `doctor.rs`'s `test_report` does.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{CheckStatus, DoctorTask, TaskCheck, TaskReadiness, readiness};
use crate::config::doctor;
use crate::config::runtime::RuntimeConfig;
use crate::render::{RenderFormat, TerminalEvent, render_event};

fn runtime_from(
    config_toml: &str,
    connections_toml: &str,
    secrets: BTreeMap<String, String>,
) -> RuntimeConfig {
    use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve};
    let config = ConfigFile::from_toml(config_toml).unwrap();
    let connections = ConnectionsFile::from_toml(connections_toml).unwrap();
    let resolved = resolve(ResolutionInput::new(connections.clone()).with_user(config)).unwrap();
    RuntimeConfig {
        resolved,
        connections,
        config_path: None,
        connections_path: None,
        cache_scope: std::path::PathBuf::from("/tmp/saya-doctor-task-test"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: secrets,
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-doctor-task-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn check_named<'a>(readiness: &'a TaskReadiness, name: &str) -> &'a TaskCheck {
    readiness
        .checks
        .iter()
        .find(|check| check.name == name)
        .unwrap_or_else(|| panic!("no `{name}` check in {readiness:?}"))
}

// --- query: readiness equals the legacy `can_run_query` decision ---

fn query_runtime(profile_toml: Option<&str>) -> RuntimeConfig {
    match profile_toml {
        Some(toml) => runtime_from(
            "default_profile = \"analytics\"\n",
            &format!("[profiles.analytics]\n{toml}"),
            BTreeMap::new(),
        ),
        None => runtime_from("", "", BTreeMap::new()),
    }
}

#[test]
fn query_readiness_equals_can_run_query_when_ready() {
    let runtime = query_runtime(Some("type = \"sqlite\"\npath = \"/tmp/x.sqlite\"\n"));
    let readiness = readiness(&runtime, DoctorTask::Query, Path::new("/"));
    assert_eq!(readiness.ready, doctor::report(&runtime).can_run_query);
    assert!(readiness.ready);
    let profile = check_named(&readiness, "profile");
    assert_eq!(profile.status, CheckStatus::Ok);
    assert!(
        profile.detail.contains("selected: analytics (sqlite)"),
        "{profile:?}"
    );
    assert_eq!(check_named(&readiness, "secrets").status, CheckStatus::Ok);
}

#[test]
fn query_readiness_equals_can_run_query_when_a_secret_does_not_resolve() {
    let runtime = query_runtime(Some(
        "type = \"postgresql\"\nhost = \"localhost\"\ndatabase = \"warehouse\"\n\
         user = \"u\"\npassword = { env = \"SAYA_TEST_UNRESOLVED\" }\n",
    ));
    let readiness = readiness(&runtime, DoctorTask::Query, Path::new("/"));
    assert_eq!(readiness.ready, doctor::report(&runtime).can_run_query);
    assert!(!readiness.ready);
    let secrets = check_named(&readiness, "secrets");
    assert_eq!(secrets.status, CheckStatus::Fail);
    assert!(
        secrets.detail.contains("env:SAYA_TEST_UNRESOLVED"),
        "{secrets:?}"
    );
}

#[test]
fn query_readiness_equals_can_run_query_when_no_profile_is_selected() {
    let runtime = query_runtime(None);
    let readiness = readiness(&runtime, DoctorTask::Query, Path::new("/"));
    assert_eq!(readiness.ready, doctor::report(&runtime).can_run_query);
    assert!(!readiness.ready);
    let profile = check_named(&readiness, "profile");
    assert_eq!(profile.status, CheckStatus::Fail);
    assert_eq!(
        profile.detail,
        "no connection profile is selected — add one to connections.toml or pass --connections/--profile"
    );
    // No selected profile means no references to resolve: vacuously fine.
    assert_eq!(check_named(&readiness, "secrets").status, CheckStatus::Ok);
}

// --- ai: provider, credentials, endpoint ---

#[test]
fn ai_without_a_key_reference_fails_credentials_for_a_cloud_provider() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    assert!(!readiness.ready);
    let credentials = check_named(&readiness, "credentials");
    assert_eq!(credentials.status, CheckStatus::Fail);
    assert!(
        credentials.detail.contains("unauthenticated"),
        "{credentials:?}"
    );
    let provider = check_named(&readiness, "provider");
    assert_eq!(provider.status, CheckStatus::Ok);
    assert_eq!(provider.detail, "openai, model gpt-4o");
}

#[test]
fn ai_credentials_fail_names_the_redacted_reference_label_not_any_value() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\napi_key = { env = \"SAYA_TEST_AI_KEY\" }\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    assert!(!readiness.ready);
    let credentials = check_named(&readiness, "credentials");
    assert_eq!(credentials.status, CheckStatus::Fail);
    assert!(
        credentials.detail.contains("env:SAYA_TEST_AI_KEY"),
        "{credentials:?}"
    );
}

#[test]
fn ai_credentials_resolve_from_secret_values_without_serializing_the_value() {
    const SENTINEL: &str = "sentinel-ai-key-never-print-this";
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\napi_key = { env = \"SAYA_TEST_AI_KEY\" }\n",
        "",
        BTreeMap::from([("SAYA_TEST_AI_KEY".to_string(), SENTINEL.to_string())]),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    assert!(readiness.ready);
    assert_eq!(
        check_named(&readiness, "credentials").status,
        CheckStatus::Ok
    );
    let serialized = serde_json::to_string(&readiness).unwrap();
    assert!(!serialized.contains(SENTINEL), "{serialized}");
}

#[test]
fn a_local_provider_is_ready_without_credentials() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"qwen2.5-coder:14b\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    assert!(readiness.ready);
    let credentials = check_named(&readiness, "credentials");
    assert_eq!(credentials.status, CheckStatus::Ok);
    assert_eq!(credentials.detail, "no credentials required for ollama");
    let endpoint = check_named(&readiness, "endpoint");
    assert_eq!(endpoint.status, CheckStatus::Ok);
    assert_eq!(endpoint.detail, "configured endpoint: localhost:11434");
}

#[test]
fn an_empty_model_fails_the_provider_check() {
    let runtime = runtime_from("[ai]\nmodel = \"\"\n", "", BTreeMap::new());
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    assert!(!readiness.ready);
    assert_eq!(
        check_named(&readiness, "provider").status,
        CheckStatus::Fail
    );
}

#[test]
fn an_endpointless_gateway_warns_without_failing() {
    // openai_compatible has no default endpoint and none configured: there is
    // no address to report, which is a warning, never a connection attempt.
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai_compatible\"\nmodel = \"m\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    let endpoint = check_named(&readiness, "endpoint");
    assert_eq!(endpoint.status, CheckStatus::Warn);
    assert_eq!(endpoint.detail, "configured endpoint: none");
    // A warning never blocks readiness: only credentials decide here.
    assert!(readiness.ready);
}

#[test]
fn an_unparseable_base_url_warns_the_endpoint_check() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"m\"\nbase_url = \"not a url\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    let endpoint = check_named(&readiness, "endpoint");
    assert_eq!(endpoint.status, CheckStatus::Warn);
    assert!(
        endpoint.detail.contains("could not be parsed"),
        "{endpoint:?}"
    );
    // The raw base_url is never echoed: it may carry userinfo.
    assert!(!endpoint.detail.contains("not a url"));
}

// --- coding: ai checks plus workspace; the database is not consulted ---

fn inside_a_worktree(start: &Path) -> bool {
    let mut current = Some(start.to_path_buf());
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            return true;
        }
        current = dir.parent().map(Path::to_path_buf);
    }
    false
}

#[test]
fn coding_is_ready_in_a_git_worktree_with_zero_profiles() {
    let dir = temp_dir("worktree");
    std::fs::create_dir(dir.join(".git")).unwrap();
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"qwen2.5-coder:14b\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Coding, &dir);
    // Zero profiles, ready AI, a worktree: coding is ready — the database is
    // NOT consulted for this task.
    assert!(readiness.ready, "{readiness:?}");
    let workspace = check_named(&readiness, "workspace");
    assert_eq!(workspace.status, CheckStatus::Ok);
    let canonical = std::fs::canonicalize(&dir).unwrap();
    assert_eq!(workspace.detail, canonical.display().to_string());
    assert_eq!(
        readiness
            .checks
            .iter()
            .map(|check| check.name.as_str())
            .collect::<Vec<_>>(),
        vec!["provider", "credentials", "endpoint", "workspace"]
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn coding_outside_any_worktree_fails_the_workspace_check() {
    let dir = temp_dir("no-worktree");
    if inside_a_worktree(&dir) {
        eprintln!(
            "skipping: {} unexpectedly sits inside a git worktree on this machine",
            dir.display()
        );
        return;
    }
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"qwen2.5-coder:14b\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Coding, &dir);
    assert!(!readiness.ready, "{readiness:?}");
    let workspace = check_named(&readiness, "workspace");
    assert_eq!(workspace.status, CheckStatus::Fail);
    assert_eq!(
        workspace.detail,
        "no project directory here — run inside a git worktree"
    );
    let _ = std::fs::remove_dir_all(dir);
}

// --- serialization shape ---

#[test]
fn readiness_serializes_as_task_ready_checks_with_snake_case_statuses() {
    let dir = temp_dir("shape");
    std::fs::create_dir(dir.join(".git")).unwrap();
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"qwen2.5-coder:14b\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Coding, &dir);
    let value: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&readiness).unwrap()).unwrap();
    assert_eq!(value["task"], "coding");
    assert_eq!(value["ready"], true);
    let checks = value["checks"].as_array().unwrap();
    assert_eq!(checks[0]["name"], "provider");
    assert_eq!(checks[0]["status"], "ok");
    assert!(checks[0]["detail"].is_string());
    assert_eq!(checks[1]["status"], "ok");
    let _ = std::fs::remove_dir_all(dir);
}

// --- the TerminalEvent text shape, through the public render seam ---

fn event_from(readiness: &TaskReadiness) -> TerminalEvent {
    TerminalEvent::DoctorTask {
        task: readiness.task.to_string(),
        ready: readiness.ready,
        checks: readiness.checks.clone(),
    }
}

#[test]
fn the_text_shape_prints_ready_and_one_line_per_check() {
    let checks = vec![
        TaskCheck {
            name: "provider".into(),
            status: CheckStatus::Ok,
            detail: "openai, model gpt-4o".into(),
            hint: None,
        },
        TaskCheck {
            name: "endpoint".into(),
            status: CheckStatus::Warn,
            detail: "configured endpoint: none".into(),
            hint: None,
        },
    ];
    let readiness = TaskReadiness {
        task: "ai",
        ready: true,
        checks,
    };
    let rendered = render_event(&event_from(&readiness), RenderFormat::Text);
    assert_eq!(
        rendered.stdout,
        "task ai: ready\n\
         ✓ provider: openai, model gpt-4o\n\
         ! endpoint: configured endpoint: none\n"
    );
}

#[test]
fn the_text_shape_marks_a_not_ready_task() {
    let checks = vec![TaskCheck {
        name: "profile".into(),
        status: CheckStatus::Fail,
        detail: "no connection profile is selected".into(),
        hint: None,
    }];
    let readiness = TaskReadiness {
        task: "query",
        ready: false,
        checks,
    };
    let rendered = render_event(&event_from(&readiness), RenderFormat::Text);
    assert_eq!(
        rendered.stdout,
        "task query: NOT ready\n\
         ✗ profile: no connection profile is selected\n"
    );
}

// --- edit hints: a failing or warning check says what to edit ---

fn user_config_file() -> String {
    crate::config::sources::user_config_dir()
        .join("config.toml")
        .display()
        .to_string()
}

#[test]
fn the_provider_fail_hint_names_the_user_config_file_and_the_model_flag() {
    let runtime = runtime_from("[ai]\nmodel = \"\"\n", "", BTreeMap::new());
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    let provider = check_named(&readiness, "provider");
    assert_eq!(provider.status, CheckStatus::Fail);
    assert_eq!(
        provider.hint.as_deref(),
        Some(format!("Set [ai] model in {}, or pass --model.", user_config_file()).as_str())
    );
}

#[test]
fn the_missing_credentials_hint_names_the_user_file_and_the_export_alternative() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    let credentials = check_named(&readiness, "credentials");
    assert_eq!(credentials.status, CheckStatus::Fail);
    assert_eq!(
        credentials.hint.as_deref(),
        Some(
            format!(
                "Set [ai] api_key = {{ env = \"NAME\" }} in {} (or export SAYA_AI_API_KEY).",
                user_config_file()
            )
            .as_str()
        )
    );
}

#[test]
fn the_unresolved_credentials_hint_names_the_variable_and_the_label_only() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\napi_key = { env = \"SAYA_TEST_AI_KEY\" }\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    let credentials = check_named(&readiness, "credentials");
    assert_eq!(credentials.status, CheckStatus::Fail);
    // The exact text: the variable name and the redacted label — a secret
    // value has nowhere to appear because the hint is fully pinned.
    assert_eq!(
        credentials.hint.as_deref(),
        Some(
            "Export SAYA_TEST_AI_KEY in your shell or --env-file; the reference is env:SAYA_TEST_AI_KEY."
        )
    );
}

#[test]
fn the_endpoint_warning_hint_points_at_the_user_config_file() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai_compatible\"\nmodel = \"m\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    let endpoint = check_named(&readiness, "endpoint");
    assert_eq!(endpoint.status, CheckStatus::Warn);
    assert_eq!(
        endpoint.hint.as_deref(),
        Some(format!("Set [ai] base_url in {}.", user_config_file()).as_str())
    );
}

#[test]
fn ready_checks_carry_no_hint_and_the_wire_omits_the_key() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"qwen2.5-coder:14b\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    assert!(readiness.ready);
    for check in &readiness.checks {
        assert_eq!(check.hint, None, "{:?}", check.name);
    }
    let serialized = serde_json::to_string(&readiness).unwrap();
    assert!(!serialized.contains("\"hint\""), "{serialized}");
}

#[test]
fn the_profile_fail_hint_names_the_setup_command() {
    let runtime = runtime_from("", "", BTreeMap::new());
    let readiness = readiness(&runtime, DoctorTask::Query, Path::new("/"));
    let profile = check_named(&readiness, "profile");
    assert_eq!(profile.status, CheckStatus::Fail);
    assert_eq!(
        profile.hint.as_deref(),
        Some(
            "Add a profile to connections.toml or pass --connections/--profile; \
             `saya setup` can do it."
        )
    );
}

#[test]
fn the_secrets_fail_hint_says_to_export_without_printing_values() {
    let runtime = runtime_from(
        "default_profile = \"analytics\"\n",
        "[profiles.analytics]\ntype = \"postgresql\"\nhost = \"localhost\"\ndatabase = \"warehouse\"\n\
         user = \"u\"\npassword = { env = \"SAYA_TEST_UNRESOLVED\" }\n",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Query, Path::new("/"));
    let secrets = check_named(&readiness, "secrets");
    assert_eq!(secrets.status, CheckStatus::Fail);
    assert_eq!(
        secrets.hint.as_deref(),
        Some("Export the variables named above; values are never printed.")
    );
}

#[test]
fn the_workspace_fail_hint_says_how_to_launch() {
    let dir = temp_dir("worktree-hint");
    if inside_a_worktree(&dir) {
        eprintln!(
            "skipping: {} unexpectedly sits inside a git worktree on this machine",
            dir.display()
        );
        return;
    }
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"m\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Coding, &dir);
    let workspace = check_named(&readiness, "workspace");
    assert_eq!(workspace.status, CheckStatus::Fail);
    assert_eq!(
        workspace.hint.as_deref(),
        Some(
            "Launch inside a git worktree or pass --workspace <dir> \
             (not available to config doctor: run saya from the project)."
        )
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_text_shape_prints_a_fix_line_under_a_check_with_a_hint() {
    let checks = vec![TaskCheck {
        name: "credentials".into(),
        status: CheckStatus::Fail,
        detail: "no api_key reference configured — requests to openai will be unauthenticated"
            .into(),
        hint: Some("Set [ai] api_key = { env = \"NAME\" } in /u/.config/saya/config.toml (or export SAYA_AI_API_KEY).".into()),
    }];
    let readiness = TaskReadiness {
        task: "ai",
        ready: false,
        checks,
    };
    let rendered = render_event(&event_from(&readiness), RenderFormat::Text);
    assert_eq!(
        rendered.stdout,
        "task ai: NOT ready\n\
         ✗ credentials: no api_key reference configured — requests to openai will be unauthenticated\n\
         \x20  fix: Set [ai] api_key = { env = \"NAME\" } in /u/.config/saya/config.toml (or export SAYA_AI_API_KEY).\n"
    );
}
