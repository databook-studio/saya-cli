use std::collections::BTreeSet;

use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve, user_facing_keys};
use serde_json::{Value, json};

fn empty() -> ResolutionInput {
    ResolutionInput::new(ConnectionsFile::default())
}

fn assert_json_fields(object: &Value, expected: &[(&str, Value)]) {
    for (field, value) in expected {
        assert_eq!(&object[*field], value, "wrong diagnostic value for {field}");
        if value.is_boolean() {
            assert!(
                object[*field].is_boolean(),
                "{field} must be a JSON boolean"
            );
        } else if value.is_number() {
            assert!(object[*field].is_number(), "{field} must be a JSON number");
        }
    }
}

#[test]
fn job_diagnostics_preserve_raw_effective_types_and_provenance_keys() {
    let absent = ConfigFile::from_toml("").expect("empty config parses");
    let raw_absent = absent.redacted_diagnostics();
    assert!(
        raw_absent.limits_job_budget_mode.is_none()
            && raw_absent.agents_max_children.is_none()
            && raw_absent.agents_max_concurrent_model_requests.is_none()
            && raw_absent.job_program_dir.is_none()
            && raw_absent.limits_job_logical_requests.is_none()
            && raw_absent.limits_job_transport_attempts.is_none()
            && raw_absent.limits_job_tool_calls.is_none()
            && raw_absent.limits_job_known_tokens.is_none()
            && raw_absent.limits_job_job_tokens.is_none()
            && raw_absent.limits_job_elapsed_seconds.is_none()
            && raw_absent.job_automatic_lsp_profile.is_none()
            && raw_absent.agents.is_none()
    );
    let raw_json = serde_json::to_value(raw_absent).unwrap();
    let missing = [
        ("limits_job_budget_mode", Value::Null),
        ("agents_max_children", Value::Null),
        ("agents_max_concurrent_model_requests", Value::Null),
        ("job_program_dir", Value::Null),
        ("limits_job_logical_requests", Value::Null),
        ("limits_job_transport_attempts", Value::Null),
        ("limits_job_tool_calls", Value::Null),
        ("limits_job_known_tokens", Value::Null),
        ("limits_job_job_tokens", Value::Null),
        ("limits_job_elapsed_seconds", Value::Null),
        ("job_automatic_lsp_profile", Value::Null),
        ("agents", Value::Null),
    ];
    assert_json_fields(&raw_json, &missing);

    let resolved_defaults = resolve(empty()).expect("default config resolves");
    let effective_defaults =
        serde_json::to_value(resolved_defaults.redacted_diagnostics()).unwrap();
    let defaults = [
        ("limits_job_budget_mode", json!("bounded")),
        ("agents_max_children", json!(3)),
        ("agents_max_concurrent_model_requests", json!(4)),
        ("job_program_dir", Value::Null),
        ("limits_job_logical_requests", json!(2_000)),
        ("limits_job_transport_attempts", json!(6_000)),
        ("limits_job_tool_calls", json!(10_000)),
        ("limits_job_known_tokens", Value::Null),
        ("limits_job_job_tokens", Value::Null),
        ("limits_job_elapsed_seconds", json!(604_800)),
        ("limits_job_no_progress_turns", json!(8)),
        ("limits_job_repeat_limit", json!(3)),
        ("limits_job_wall_clock_seconds", Value::Null),
        ("limits_job_max_cost_usd", Value::Null),
        ("job_automatic_lsp_profile", Value::Null),
        (
            "agents",
            json!({
                "explore": {"role": "analyst", "allow_write": false, "model": null},
                "general": {"role": "implementer", "allow_write": true, "model": null},
            }),
        ),
    ];
    assert_json_fields(&effective_defaults, &defaults);

    let file = ConfigFile::from_toml(
        "[job]\nprogram_dir = '/opt/saya-programs'\n\n[limits.job]\nbudget_mode = 'bounded'\nlogical_requests = 40\ntransport_attempts = 60\ntool_calls = 123\nknown_tokens = 23456\njob_tokens = 345678\nelapsed_seconds = 900\n\n[job.extensions.lookup]\nprotocol = 'mcp'\nprogram = 'extension-server'\nargv = ['private-extension-arg']\n\n[agents]\nmax_children = 2\nmax_concurrent_model_requests = 3\n\n[agents.reviewer]\nrole = 'analyst'\nmodel = 'reader-model'\n",
    )
    .expect("[job] values parse");
    let raw_user = serde_json::to_value(file.redacted_diagnostics()).unwrap();
    let resolved_user = resolve(empty().with_user(file)).expect("user config resolves");
    let effective_user = serde_json::to_value(resolved_user.redacted_diagnostics()).unwrap();
    let user_values = [
        ("limits_job_budget_mode", json!("bounded")),
        ("agents_max_children", json!(2)),
        ("agents_max_concurrent_model_requests", json!(3)),
        ("job_program_dir", json!("/opt/saya-programs")),
        ("limits_job_logical_requests", json!(40)),
        ("limits_job_transport_attempts", json!(60)),
        ("limits_job_tool_calls", json!(123)),
        ("limits_job_known_tokens", json!(23_456)),
        ("limits_job_job_tokens", json!(345_678)),
        ("limits_job_elapsed_seconds", json!(900)),
        ("job_automatic_lsp_profile", Value::Null),
        (
            "job_extensions",
            json!({"lookup": {
                "protocol": "mcp",
                "program": "extension-server",
                "argv_count": 1,
                "language": null,
                "tool_policy_count": 0,
            }}),
        ),
    ];
    assert_json_fields(&raw_user, &user_values);
    assert_json_fields(&effective_user, &user_values);
    // The raw view carries only the fields the entry stated; the effective one
    // carries every type the job advertises, built-ins included.
    assert_eq!(
        raw_user["agents"],
        json!({"reviewer": {"role": "analyst", "model": "reader-model"}})
    );
    assert_eq!(
        effective_user["agents"],
        json!({
            "explore": {"role": "analyst", "allow_write": false, "model": null},
            "general": {"role": "implementer", "allow_write": true, "model": null},
            "reviewer": {"role": "analyst", "allow_write": false, "model": "reader-model"},
        })
    );

    let expected_keys = [
        ("job_extensions", "job.extensions"),
        ("limits_job_budget_mode", "limits.job.budget_mode"),
        ("agents_max_children", "agents.max_children"),
        (
            "agents_max_concurrent_model_requests",
            "agents.max_concurrent_model_requests",
        ),
        ("job_program_dir", "job.program_dir"),
        ("job_root_mode", "job.root_mode"),
        ("limits_job_logical_requests", "limits.job.logical_requests"),
        (
            "limits_job_transport_attempts",
            "limits.job.transport_attempts",
        ),
        ("limits_job_tool_calls", "limits.job.tool_calls"),
        ("limits_job_known_tokens", "limits.job.known_tokens"),
        ("limits_job_job_tokens", "limits.job.job_tokens"),
        ("limits_job_elapsed_seconds", "limits.job.elapsed_seconds"),
        (
            "limits_job_no_progress_turns",
            "limits.job.no_progress_turns",
        ),
        ("limits_job_repeat_limit", "limits.job.repeat_limit"),
        (
            "limits_job_wall_clock_seconds",
            "limits.job.wall_clock_seconds",
        ),
        ("limits_job_max_cost_usd", "limits.job.max_cost_usd"),
        ("job_automatic_lsp_profile", "job.automatic_lsp_profile"),
        ("agents", "agents"),
    ];
    let actual_keys: BTreeSet<_> = user_facing_keys()
        .iter()
        .copied()
        .filter(|(field, _)| {
            field.starts_with("job_")
                || field.starts_with("limits_job_")
                || *field == "agents"
                || field.starts_with("agents_")
        })
        .collect();
    assert_eq!(actual_keys, expected_keys.into_iter().collect());
    for (_, dotted_key) in expected_keys {
        assert!(
            resolved_user.provenance.get(dotted_key).is_some(),
            "missing provenance for {dotted_key}"
        );
    }
}

#[test]
fn job_effective_diagnostics_follow_untrusted_project_reversion() {
    let user = ConfigFile::from_toml("[agents]\nmax_children = 2\n").unwrap();
    let project = ConfigFile::from_toml("[agents]\nmax_children = 1\n").unwrap();
    let raw_project = serde_json::to_value(project.redacted_diagnostics()).unwrap();
    assert_eq!(raw_project["agents_max_children"], json!(1));

    let resolved = resolve(empty().with_user(user).with_project(project)).unwrap();
    assert_eq!(resolved.ignored_project_overrides, ["agents"]);
    let effective = serde_json::to_value(resolved.redacted_diagnostics()).unwrap();
    assert_eq!(effective["agents_max_children"], json!(2));
}
