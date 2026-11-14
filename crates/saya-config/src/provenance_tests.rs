use crate::{
    ConnectionsFile, ResolutionInput, SecretRef,
    provenance::{Recorder, SettingOrigin, SettingProvenance, SettingSource, USER_FACING},
    resolve,
};

use super::apply_user_facing_defaults;

/// A user file that states every setting the merge funnels can record, so
/// the drift guard can prove every `USER_FACING` key is attributed.
const FULL_USER_TOML: &str = "
default_profile = 'local'
[ai]
provider = 'openai'
model = 'test-model'
base_url = 'https://user.test/v1'
allow_data_sharing = true
app_attribution = false
api_key = { env = 'SAYA_TEST_KEY' }
temperature = 0.5
timeout_seconds = 61
idle_timeout_seconds = 91
request_timeout_seconds = 181
max_output_tokens = 100
reasoning_effort = 'high'
retry_delays_ms = [100, 200]
context_byte_budget = 4096
context_window_tokens = 8192
show_thinking = true
compaction = 'manual'
compaction_trigger_percent = 50
[[ai.endpoints]]
name = 'extra'
provider = 'openai'
model = 'endpoint-model'
base_url = 'https://extra.test/v1'
api_key = { env = 'SAYA_EXTRA_KEY' }
[limits.session]
requests = 8
tool_calls = 12
elapsed_seconds = 45
known_tokens = 9000
[limits.ask]
requests = 10
tool_calls = 20
elapsed_seconds = 60
known_tokens = 30000
[limits.run]
wall_clock_seconds = 3600
turns = 5
tool_calls = 20
no_progress_turns = 4
repeat_limit = 5
[limits.run.tokens_per_endpoint]
extra = 1000
[run]
read_only = false
max_rows = 50
candidates = 2
query_timeout_seconds = 30
[jobs.fetch]
max_file_bytes = 1024
max_run_bytes = 4096
timeout_seconds = 10
[job]
program_dir = '/saya-test/programs'
[agents]
max_children = 2
max_concurrent_model_requests = 3
[agents.reviewer]
role = 'analyst'
[tools]
allow = ['read', 'grep']
[permissions]
allow_bypass = true
[host_commands]
pass_env = ['PATH']
timeout_seconds = 120
[session_commands]
deny = ['rm']
[output]
format = 'json'
color = 'always'
[memory]
mode = 'assisted'
max_contracts = 6
max_claims_per_contract = 10
max_context_bytes = 4096
[instructions]
project = false
personal = true
project_max_bytes = 1024
personal_max_bytes = 512
total_max_bytes = 2048
[ui]
theme = 'dark'
";

/// Drift guard: every key in `USER_FACING` is one the merge funnels record.
/// A fully-populated user file attributes every user-facing key to
/// `UserFile`; an empty input attributes every one to `Default`.
#[test]
fn every_user_facing_key_is_recorded_by_the_merge() {
    let connections =
        ConnectionsFile::from_toml("[profiles.local]\ntype = 'sqlite'\npath = ':memory:'\n")
            .unwrap();
    let resolved = resolve(
        ResolutionInput::new(connections)
            .with_user(crate::ConfigFile::from_toml(FULL_USER_TOML).unwrap()),
    )
    .unwrap();
    for key in USER_FACING {
        let origin = resolved
            .provenance
            .get(key)
            .unwrap_or_else(|| panic!("no origin recorded for {key}"));
        assert_eq!(origin.source, SettingSource::UserFile, "{key}");
        if *key != "ai.api_key" {
            assert_eq!(origin.reference, None, "{key} carries no reference label");
        }
    }
    assert_eq!(
        resolved
            .provenance
            .get("ai.api_key")
            .unwrap()
            .reference
            .as_deref(),
        Some("env:SAYA_TEST_KEY")
    );
    // A declared endpoint's supplied fields are attributed to the layer
    // that supplied them.
    for field in ["provider", "model", "base_url", "api_key"] {
        let key = format!("ai.endpoints[\"extra\"].{field}");
        assert_eq!(
            resolved.provenance.get(&key).map(|origin| &origin.source),
            Some(&SettingSource::UserFile),
            "{key}"
        );
    }
    // The orchestrator fallback is synthesized at resolution, not merged:
    // its fields carry no recorded origin and report Default.
    for field in ["provider", "model", "base_url", "api_key"] {
        let key = format!("ai.endpoints[\"orchestrator\"].{field}");
        assert_eq!(
            resolved.provenance.get(&key).map(|origin| &origin.source),
            Some(&SettingSource::Default),
            "{key}"
        );
    }

    let resolved = resolve(ResolutionInput::new(ConnectionsFile::default())).unwrap();
    for key in USER_FACING {
        let origin = resolved
            .provenance
            .get(key)
            .unwrap_or_else(|| panic!("no default origin for {key}"));
        assert_eq!(origin.source, SettingSource::Default, "{key}");
        assert_eq!(origin.reference, None, "{key}");
    }
}

#[test]
fn provenance_serializes_as_a_value_free_map() {
    let mut recorder = Recorder::default();
    recorder.record("ai.model", SettingSource::UserFile);
    recorder.record_secret(
        "ai.api_key",
        SettingSource::Env {
            var: "SAYA_KEY".into(),
        },
        &SecretRef::Env {
            env: "SAYA_KEY".into(),
        },
    );
    let json = serde_json::to_value(recorder.finish()).unwrap();
    assert_eq!(
        json["ai.model"],
        serde_json::json!({ "source": { "layer": "user_file" } })
    );
    assert_eq!(
        json["ai.api_key"],
        serde_json::json!({
            "source": { "layer": "env", "var": "SAYA_KEY" },
            "reference": "env:SAYA_KEY"
        })
    );
}

/// Drift guard: every `[limits]` key is a user-facing setting. It must have a
/// row in the `--explain` table and a `Default` origin in the fill, or
/// `config show --explain` silently omits a ceiling a user set.
#[test]
fn every_limits_key_is_explained_and_defaulted() {
    const LIMITS_KEYS: &[&str] = &[
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
        "limits.run.no_progress_turns",
        "limits.run.repeat_limit",
        "limits.job.budget_mode",
        "limits.job.logical_requests",
        "limits.job.transport_attempts",
        "limits.job.tool_calls",
        "limits.job.known_tokens",
        "limits.job.job_tokens",
        "limits.job.elapsed_seconds",
        "limits.job.no_progress_turns",
        "limits.job.repeat_limit",
        "limits.job.wall_clock_seconds",
        "limits.job.max_cost_usd",
    ];
    let explained: Vec<&str> = crate::user_facing_keys()
        .iter()
        .map(|(_, dotted)| *dotted)
        .collect();
    let resolved = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("the default config resolves");
    for key in LIMITS_KEYS {
        assert!(
            explained.contains(key),
            "{key} has no `config show --explain` row"
        );
        let origin = resolved
            .provenance
            .get(key)
            .unwrap_or_else(|| panic!("no default origin for {key}"));
        assert_eq!(origin.source, SettingSource::Default, "{key}");
    }
}

#[test]
fn rollback_restores_reverted_keys_and_drops_project_only_entries() {
    let mut recorder = Recorder::default();
    recorder.record("ai.base_url", SettingSource::UserFile);
    let snapshot = recorder.snapshot();
    recorder.record("ai.base_url", SettingSource::ProjectFile);
    recorder.record(
        "ai.endpoints[\"injected\"].base_url",
        SettingSource::ProjectFile,
    );
    recorder.rollback(
        &[
            "ai.base_url".to_owned(),
            "ai.endpoints[\"injected\"]".to_owned(),
        ],
        &snapshot,
    );
    assert_eq!(
        recorder
            .origin_of("ai.base_url")
            .map(|origin| &origin.source),
        Some(&SettingSource::UserFile)
    );
    assert_eq!(
        recorder.origin_of("ai.endpoints[\"injected\"].base_url"),
        None
    );
}

#[test]
fn setting_provenance_lookups_round_trip() {
    let mut provenance = SettingProvenance::default();
    apply_user_facing_defaults(&mut provenance, &Default::default());
    // Every user-facing key is filled; with no declared endpoints, no
    // endpoint keys are.
    assert_eq!(provenance.iter().count(), USER_FACING.len());
    assert_eq!(
        provenance.get("ai.model").map(|origin| &origin.source),
        Some(&SettingSource::Default)
    );
    assert_eq!(provenance.get("ai.endpoints[\"orchestrator\"].model"), None);
    let origin = SettingOrigin {
        source: SettingSource::Cli {
            flag: "--max-rows".into(),
        },
        reference: None,
    };
    provenance
        .0
        .insert("run.max_rows".to_owned(), origin.clone());
    assert_eq!(provenance.get("run.max_rows"), Some(&origin));
    assert_eq!(
        provenance.get("run.read_only").map(|origin| &origin.source),
        Some(&SettingSource::Default),
        "the fill stays; the insert overrides only its own key"
    );
}
