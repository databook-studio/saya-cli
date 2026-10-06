//! Tests for the configuration reload core: a candidate built without side
//! effects, a plan that classifies every change, and re-seeds only what the
//! user never overrode. Authority-sensitive settings (data sharing, host
//! command shaping) must never move for a running session.

use super::classify::{CLASS_TABLE, Class, DEFAULTS_TO_PENDING, classify_change, classify_key};
use super::plan::{ChangeLine, ReloadPlan, StateReseed};
use super::{ReloadInputs, apply_plan, build_candidate, plan_reload};
use crate::cli::GlobalOptions;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::SessionState;
use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn resolved_from(
    user_toml: &str,
    project_toml: Option<&str>,
    connections_toml: &str,
) -> saya_config::ResolvedConfig {
    let connections = ConnectionsFile::from_toml(connections_toml).unwrap();
    let mut input =
        ResolutionInput::new(connections).with_user(ConfigFile::from_toml(user_toml).unwrap());
    if let Some(project) = project_toml {
        input = input.with_project(ConfigFile::from_toml(project).unwrap());
    }
    resolve(input).unwrap()
}

fn runtime_from(config_toml: &str, connections_toml: &str) -> RuntimeConfig {
    runtime_from_layers(config_toml, None, connections_toml)
}

fn runtime_from_layers(
    user_toml: &str,
    project_toml: Option<&str>,
    connections_toml: &str,
) -> RuntimeConfig {
    RuntimeConfig {
        resolved: resolved_from(user_toml, project_toml, connections_toml),
        connections: ConnectionsFile::from_toml(connections_toml).unwrap(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-reload-test"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: BTreeMap::new(),
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-reload-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn state_with(model: &str) -> SessionState {
    SessionState::new("s1", None, model)
}

// --- build_candidate ---

#[test]
fn candidate_reloads_the_real_sources_and_the_process_env_map_wins() {
    let user = temp_dir("user");
    std::fs::write(user.join("config.toml"), "[ai]\nmodel = \"from-file\"\n").unwrap();
    let cwd = temp_dir("cwd");
    let inputs = ReloadInputs {
        options: GlobalOptions::default(),
        cwd,
        user_dir: user,
        process_env: BTreeMap::from([("SAYA_MODEL".to_owned(), "from-env".to_owned())]),
    };
    let candidate = build_candidate(&inputs).unwrap();
    assert_eq!(candidate.resolved.ai.model, "from-env");
    assert!(candidate.resolved.ignored_project_overrides.is_empty());
}

#[test]
fn a_failed_build_leaves_the_callers_runtime_alone() {
    let user = temp_dir("invalid");
    std::fs::write(user.join("config.toml"), "[ai]\nmodel = \"m1\"\n").unwrap();
    let cwd = temp_dir("invalid-cwd");
    let inputs = ReloadInputs {
        options: GlobalOptions::default(),
        cwd,
        user_dir: user.clone(),
        process_env: BTreeMap::new(),
    };
    let held = build_candidate(&inputs).unwrap();
    std::fs::write(user.join("config.toml"), "[ai]\nmodel = \"unterminated\n").unwrap();
    let error = build_candidate(&inputs).unwrap_err();
    assert!(!error.is_empty());
    assert_eq!(
        held.resolved.ai.model, "m1",
        "the failed rebuild must not touch the runtime the caller already holds"
    );
}

#[test]
fn build_candidate_rereads_the_env_file_every_call() {
    let user = temp_dir("envfile-user");
    std::fs::write(user.join("config.toml"), "").unwrap();
    let env_file = temp_dir("envfile").join("overrides.env");
    std::fs::write(&env_file, "SAYA_AI_MODEL=first\n").unwrap();
    let inputs = ReloadInputs {
        options: GlobalOptions {
            env_file: Some(env_file),
            ..Default::default()
        },
        cwd: temp_dir("envfile-cwd"),
        user_dir: user,
        process_env: BTreeMap::new(),
    };
    let first = build_candidate(&inputs).unwrap();
    assert_eq!(first.resolved.ai.model, "first");
    std::fs::write(first_resolved_env_path(&inputs), "SAYA_AI_MODEL=second\n").unwrap();
    let second = build_candidate(&inputs).unwrap();
    assert_ne!(first.secret_values, second.secret_values);
    assert_eq!(
        second
            .secret_values
            .get("SAYA_AI_MODEL")
            .map(String::as_str),
        Some("second")
    );
}

fn first_resolved_env_path(inputs: &ReloadInputs) -> PathBuf {
    inputs.options.env_file.clone().unwrap()
}

// --- plan_reload: unchanged and section membership ---

#[test]
fn an_unchanged_runtime_yields_an_empty_plan_and_the_unchanged_line() {
    let old = runtime_from("[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 50\n", "");
    let state = state_with("m1");
    let plan = plan_reload(&old, &old, &state);
    assert!(plan.is_empty());
    assert_eq!(plan.render(), "Configuration is unchanged.");
}

#[test]
fn live_changes_land_in_applied_only() {
    let old = runtime_from(
        "[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 50\nread_only = true\n",
        "",
    );
    let new = runtime_from(
        "[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 100\nread_only = false\n",
        "",
    );
    let plan = plan_reload(&old, &new, &state_with("m1"));
    assert!(plan.reseeded.is_empty() && plan.pending.is_empty());
    let keys: Vec<&str> = plan.applied.iter().map(|line| line.key.as_str()).collect();
    assert_eq!(keys, ["max_rows", "read_only"]);
    assert_eq!(plan.reseed, StateReseed::default());
    assert_eq!(
        plan.applied
            .iter()
            .find(|l| l.key == "max_rows")
            .unwrap()
            .after,
        json!(100)
    );
}

#[test]
fn a_never_overridden_model_is_reseeded() {
    let old = runtime_from("[ai]\nmodel = \"m1\"\n", "");
    let new = runtime_from("[ai]\nmodel = \"m2\"\n", "");
    let plan = plan_reload(&old, &new, &state_with("m1"));
    assert!(plan.pending.is_empty());
    assert_eq!(plan.reseeded[0].key, "model");
    assert_eq!(plan.reseed.model.as_deref(), Some("m2"));
    let keys: Vec<&str> = plan.applied.iter().map(|line| line.key.as_str()).collect();
    assert_eq!(
        keys,
        ["endpoints"],
        "the orchestrator endpoint mirrors [ai]"
    );
    let mut session = state_with("m1");
    apply_plan(&plan, &mut session);
    assert_eq!(session.model, "m2");
}

#[test]
fn a_never_overridden_provider_is_reseeded() {
    let old = runtime_from("", "");
    let new = runtime_from("[ai]\nprovider = \"openai\"\n", "");
    let plan = plan_reload(&old, &new, &state_with("m"));
    assert_eq!(plan.reseeded[0].key, "provider");
    assert_eq!(plan.reseed.provider.as_deref(), Some("openai"));
    let mut session = state_with("m");
    apply_plan(&plan, &mut session);
    assert_eq!(session.provider, "openai");
}

#[test]
fn an_overridden_provider_stays_pending_and_the_state_is_untouched() {
    let old = runtime_from("", "");
    let new = runtime_from("[ai]\nprovider = \"openai\"\n", "");
    let mut session = state_with("m");
    session.provider = "anthropic".to_owned();
    let plan = plan_reload(&old, &new, &session);
    assert!(plan.reseeded.is_empty());
    assert_eq!(plan.pending.len(), 1);
    assert_eq!(plan.pending[0].key, "provider");
    assert_eq!(
        plan.pending[0].note.as_deref(),
        Some("this session overrides it (use /provider, /model or /connect to change it here)")
    );
    let keys: Vec<&str> = plan.applied.iter().map(|line| line.key.as_str()).collect();
    assert_eq!(
        keys,
        ["endpoints"],
        "only the endpoint pool mirrors the change"
    );
    assert_eq!(plan.reseed, StateReseed::default());
    let mut after = session.clone();
    apply_plan(&plan, &mut after);
    assert_eq!(after.provider, "anthropic");
}

#[test]
fn a_provider_already_at_the_new_value_is_omitted_from_the_reseed() {
    let old = runtime_from("", "");
    let new = runtime_from("[ai]\nprovider = \"openai\"\n", "");
    let mut session = state_with("m");
    session.provider = "openai".to_owned();
    let plan = plan_reload(&old, &new, &session);
    assert!(
        !plan.applied.is_empty() || !plan.pending.is_empty() || !plan.reseeded.is_empty(),
        "the endpoint pool still changed, so the plan is not empty"
    );
    assert!(
        !plan
            .applied
            .iter()
            .chain(&plan.pending)
            .chain(&plan.reseeded)
            .any(|line| line.key == "provider")
    );
    assert_eq!(plan.reseed.provider, None);
}

#[test]
fn data_sharing_never_moves_for_a_running_session() {
    let off = runtime_from("[ai]\nallow_data_sharing = false\n", "");
    let on = runtime_from("[ai]\nallow_data_sharing = true\n", "");
    let state = state_with("m");
    let widen = plan_reload(&off, &on, &state);
    assert_eq!(widen.pending.len(), 1);
    assert_eq!(widen.pending[0].key, "allow_data_sharing");
    assert_eq!(
        widen.pending[0].note.as_deref(),
        Some(
            "data sharing is fixed for this session; use /privacy to change it here, or start a new session"
        )
    );
    assert_eq!(widen.reseed, StateReseed::default());
    let mut session = state.clone();
    apply_plan(&widen, &mut session);
    assert!(
        !session.allow_data_sharing,
        "a reload must not widen sharing"
    );
    let narrow = plan_reload(&on, &off, &state);
    assert_eq!(narrow.pending[0].key, "allow_data_sharing");
    let mut sharing = state.clone();
    sharing.allow_data_sharing = true;
    apply_plan(&narrow, &mut sharing);
    assert!(
        sharing.allow_data_sharing,
        "a reload must not narrow sharing"
    );
}

#[test]
fn host_command_shaping_waits_for_a_new_session() {
    let old = runtime_from("", "");
    let new = runtime_from("[host_commands]\ntimeout_seconds = 99\n", "");
    let plan = plan_reload(&old, &new, &state_with("m"));
    assert_eq!(plan.pending.len(), 1);
    assert_eq!(plan.pending[0].key, "host_commands");
    assert_eq!(
        plan.pending[0].note.as_deref(),
        Some("takes effect in a new session")
    );
}

#[test]
fn session_denies_wait_for_a_new_session() {
    let old = runtime_from("", "");
    let new = runtime_from("[session_commands]\ndeny = [\"curl\"]\n", "");
    let plan = plan_reload(&old, &new, &state_with("m"));
    assert_eq!(plan.pending.len(), 1);
    assert_eq!(plan.pending[0].key, "session_commands");
    assert_eq!(
        plan.pending[0].note.as_deref(),
        Some("takes effect in a new session")
    );
}

#[test]
fn a_new_profile_is_one_connections_change_and_reseeds_the_profile_copy() {
    let old = runtime_from("", "");
    let new = runtime_from(
        "",
        "[profiles.analytics]\ntype = \"sqlite\"\npath = \"/tmp/x.sqlite\"\n",
    );
    let plan = plan_reload(&old, &new, &state_with("m"));
    assert_eq!(plan.applied.len(), 1);
    assert_eq!(plan.applied[0].key, "connections");
    assert_eq!(plan.applied[0].before, json!([]));
    assert_eq!(plan.applied[0].after, json!(["analytics"]));
    assert_eq!(plan.reseed.profile, Some(Some("analytics".to_owned())));
    let mut session = state_with("m");
    apply_plan(&plan, &mut session);
    assert_eq!(session.profile.as_deref(), Some("analytics"));
}

#[test]
fn an_unbound_endpoint_follows_the_new_configuration() {
    let old = runtime_from("[ai]\nbase_url = \"http://old:8080/v1\"\n", "");
    let new = runtime_from("[ai]\nbase_url = \"http://new:8080/v1\"\n", "");
    let plan = plan_reload(&old, &new, &state_with("m"));
    assert_eq!(plan.reseeded[0].key, "base_url");
    assert_eq!(
        plan.reseed.provider_endpoint,
        Some(Some("http://new:8080/v1".to_owned()))
    );
    let mut session = state_with("m");
    apply_plan(&plan, &mut session);
    assert_eq!(
        session.provider_endpoint.as_deref(),
        Some("http://new:8080/v1")
    );
    assert!(session.provider_endpoint_bound);
}

#[test]
fn an_overridden_endpoint_stays_pending() {
    let old = runtime_from("[ai]\nbase_url = \"http://old:8080/v1\"\n", "");
    let new = runtime_from("[ai]\nbase_url = \"http://new:8080/v1\"\n", "");
    let mut session = state_with("m");
    session.provider_endpoint = Some("http://user:9/v1".to_owned());
    session.provider_endpoint_bound = true;
    let plan = plan_reload(&old, &new, &session);
    assert_eq!(plan.pending.len(), 1);
    assert_eq!(plan.pending[0].key, "base_url");
    assert_eq!(plan.reseed, StateReseed::default());
    let mut after = session.clone();
    apply_plan(&plan, &mut after);
    assert_eq!(after.provider_endpoint.as_deref(), Some("http://user:9/v1"));
}

#[test]
fn an_endpoint_already_at_the_new_value_is_omitted_from_the_reseed() {
    let old = runtime_from("[ai]\nbase_url = \"http://old:8080/v1\"\n", "");
    let new = runtime_from("[ai]\nbase_url = \"http://new:8080/v1\"\n", "");
    let mut session = state_with("m");
    session.provider_endpoint = Some("http://new:8080/v1".to_owned());
    session.provider_endpoint_bound = true;
    let plan = plan_reload(&old, &new, &session);
    assert!(
        !plan
            .applied
            .iter()
            .chain(&plan.pending)
            .chain(&plan.reseeded)
            .any(|line| line.key == "base_url")
    );
    assert_eq!(plan.reseed.provider_endpoint, None);
}

#[test]
fn ignored_project_overrides_are_carried_and_rendered_once() {
    let old = runtime_from_layers("[run]\nmax_rows = 50\n", None, "");
    let new = runtime_from_layers(
        "[run]\nmax_rows = 50\n",
        Some("[ai]\nbase_url = \"http://project:8080/v1\"\n"),
        "",
    );
    let plan = plan_reload(&old, &new, &state_with("m"));
    assert_eq!(
        plan.ignored_project_overrides,
        vec!["ai.base_url".to_owned()]
    );
    assert!(
        plan.is_empty(),
        "a reverted project override changed no value, so only the ignored line renders"
    );
    let render = plan.render();
    assert!(render.starts_with("Configuration is unchanged."));
    assert_eq!(
        render
            .matches("Ignored from the project file: ai.base_url")
            .count(),
        1
    );
    assert!(render.contains("(security-critical; see saya config show --explain)."));
}

#[test]
fn a_secret_never_reaches_the_plan_or_the_render() {
    let user = temp_dir("sentinel");
    std::fs::write(user.join("config.toml"), "").unwrap();
    let inputs = ReloadInputs {
        options: GlobalOptions::default(),
        cwd: temp_dir("sentinel-cwd"),
        user_dir: user,
        process_env: BTreeMap::from([(
            "SAYA_AI_API_KEY".to_owned(),
            "sk-live-sentinel-123".to_owned(),
        )]),
    };
    let candidate = build_candidate(&inputs).unwrap();
    let old = runtime_from("", "");
    let plan = plan_reload(&old, &candidate, &state_with("m"));
    for line in plan
        .applied
        .iter()
        .chain(&plan.reseeded)
        .chain(&plan.pending)
    {
        let json = format!("{} {}", line.before, line.after);
        assert!(!json.contains("sk-live-sentinel-123"), "{json}");
    }
    let render = plan.render();
    assert!(!render.contains("sk-live-sentinel-123"));
    assert!(render.contains("env:SAYA_AI_API_KEY"));
}

#[test]
fn a_mixed_plan_renders_the_three_sections_in_key_order() {
    let connections = "[profiles.a]\ntype = \"sqlite\"\npath = \"/tmp/a.sqlite\"\n[profiles.b]\ntype = \"sqlite\"\npath = \"/tmp/b.sqlite\"\n";
    let old = runtime_from(
        "default_profile = \"a\"\n[ai]\nallow_data_sharing = false\n[run]\nmax_iterations = 12\nmax_rows = 50\n",
        connections,
    );
    let new = runtime_from(
        "default_profile = \"b\"\n[ai]\nallow_data_sharing = true\n[run]\nmax_iterations = 20\nmax_rows = 100\n[host_commands]\ntimeout_seconds = 99\n",
        connections,
    );
    let mut session = state_with("m1");
    session.profile = Some("a".to_owned());
    let plan = plan_reload(&old, &new, &session);
    assert_eq!(
        plan.render(),
        "Reloaded configuration: 2 applied, 1 re-seeded, 2 pending.\n\
         Applied from the next turn:\n\
         \x20 max_iterations: 12 -> 20\n\
         \x20 max_rows: 50 -> 100\n\
         Updated for this session:\n\
         \x20 profile_name: a -> b\n\
         Not applied in this session:\n\
         \x20 allow_data_sharing: false -> true — data sharing is fixed for this session; use /privacy to change it here, or start a new session\n\
         \x20 host_commands: {\"pass_env\":[],\"timeout_seconds\":600} -> {\"pass_env\":[],\"timeout_seconds\":99} — takes effect in a new session"
    );
}

// --- the class table ---

#[test]
fn the_class_table_covers_every_diagnostics_field() {
    let value = serde_json::to_value(resolved_from("", None, "").redacted_diagnostics()).unwrap();
    let keys: BTreeSet<String> = value.as_object().unwrap().keys().cloned().collect();
    let table: BTreeSet<&str> = CLASS_TABLE.iter().map(|(name, _)| *name).collect();
    for key in DEFAULTS_TO_PENDING {
        assert!(
            !table.contains(key),
            "{key} must stay out of the table so it defaults to pending"
        );
        assert!(
            keys.contains(*key),
            "{key} is no longer a diagnostics field"
        );
    }
    let unclassified: Vec<&String> = keys
        .iter()
        .filter(|key| !table.contains(key.as_str()) && !DEFAULTS_TO_PENDING.contains(&key.as_str()))
        .collect();
    assert!(
        unclassified.is_empty(),
        "new diagnostics fields need a conscious classification: {unclassified:?}"
    );
    for name in &table {
        assert!(
            keys.contains(*name),
            "class table key {name} is not a diagnostics field"
        );
    }
}

#[test]
fn an_unknown_diagnostics_key_defaults_to_pending() {
    assert!(matches!(
        classify_key("some_future_field"),
        Class::Unrecognised
    ));
    let old = runtime_from("", "");
    let new = runtime_from("", "");
    let mut plan = ReloadPlan::default();
    let change = ChangeLine::new("some_future_field", &Value::from(1), &Value::from(2), None);
    classify_change(&change, &old, &new, &state_with("m"), &mut plan);
    assert_eq!(plan.pending.len(), 1);
    assert_eq!(plan.pending[0].key, "some_future_field");
}

#[test]
fn from_launch_reads_the_current_environment() {
    let inputs = ReloadInputs::from_launch(&GlobalOptions::default());
    assert_eq!(inputs.cwd, PathBuf::from("."));
    assert_eq!(
        inputs
            .user_dir
            .file_name()
            .map(std::ffi::OsStr::to_string_lossy),
        Some(std::borrow::Cow::Borrowed("saya"))
    );
    assert!(
        !inputs.process_env.is_empty(),
        "production reads the real env"
    );
}
