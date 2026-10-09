//! Role-to-endpoint binding: a named role binds the endpoint it names, an
//! unbound role falls back to the orchestrator endpoint, a binding to an
//! endpoint outside the pool is refused, and two roles upper-casing to one
//! env name are refused.

use super::{endpoint, temp_root};
use saya_harness::endpoints::{RunConfigError, write_run_configs};
use saya_harness::run_dir::RunDir;
use saya_types::{EndpointBindings, RunId, SecretRef};

#[test]
fn a_named_role_binds_to_the_endpoint_it_names() {
    let runs_root = temp_root("named-role");
    let run = RunDir::create(&runs_root, &RunId::parse("bind-1").unwrap()).unwrap();

    let endpoints = vec![
        endpoint(
            "primary",
            "model-primary",
            Some(SecretRef::Env {
                env: "ANTHROPIC_API_KEY".to_string(),
            }),
        ),
        endpoint("orchestrator", "model-default", None),
    ];
    let bindings = EndpointBindings::new([("orchestrator", "primary")]).unwrap();

    write_run_configs(&run, &endpoints, &bindings, &[]).unwrap();

    let config = std::fs::read_to_string(run.state().join("config/config.toml")).unwrap();
    assert!(config.contains("role = \"orchestrator\""));
    assert!(config.contains("model = \"model-primary\""));
    assert!(config.contains("api_key = { env = \"SAYA_RUN_EP_ORCHESTRATOR\" }"));
    assert!(
        !config.contains("model = \"model-default\""),
        "only the bound endpoint is written, not the whole pool"
    );

    let _ = std::fs::remove_dir_all(runs_root);
}

#[test]
fn a_role_without_a_binding_falls_back_to_the_orchestrator_endpoint() {
    let runs_root = temp_root("fallback-role");
    let run = RunDir::create(&runs_root, &RunId::parse("bind-2").unwrap()).unwrap();

    let endpoints = vec![
        endpoint(
            "reviewer-ep",
            "model-reviewer",
            Some(SecretRef::Env {
                env: "REVIEWER_API_KEY".to_string(),
            }),
        ),
        endpoint("orchestrator", "model-default", None),
    ];
    let bindings = EndpointBindings::new([("reviewer", "reviewer-ep")]).unwrap();

    write_run_configs(&run, &endpoints, &bindings, &[]).unwrap();

    let config = std::fs::read_to_string(run.state().join("config/config.toml")).unwrap();
    assert!(config.contains("role = \"reviewer\""));
    assert!(config.contains("model = \"model-reviewer\""));
    assert!(config.contains("api_key = { env = \"SAYA_RUN_EP_REVIEWER\" }"));
    assert!(config.contains("role = \"orchestrator\""));
    assert!(config.contains("model = \"model-default\""));
    assert!(
        !config.contains("api_key = { env = \"SAYA_RUN_EP_ORCHESTRATOR\" }"),
        "an endpoint without a key binds nothing"
    );

    let _ = std::fs::remove_dir_all(runs_root);
}

#[test]
fn binding_to_an_endpoint_outside_the_pool_is_rejected() {
    let runs_root = temp_root("unknown-endpoint");
    let run = RunDir::create(&runs_root, &RunId::parse("bind-3").unwrap()).unwrap();

    let endpoints = vec![endpoint("primary", "model", None)];
    let bindings = EndpointBindings::new([("orchestrator", "missing")]).unwrap();

    let refused = write_run_configs(&run, &endpoints, &bindings, &[]);
    match refused {
        Err(RunConfigError::UnknownEndpoint { role, endpoint }) => {
            assert_eq!(role, "orchestrator");
            assert_eq!(endpoint, "missing");
        }
        other => panic!("expected UnknownEndpoint, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(runs_root);
}

#[test]
fn roles_sharing_one_env_binding_are_rejected() {
    let runs_root = temp_root("env-collision");
    let run = RunDir::create(&runs_root, &RunId::parse("bind-4").unwrap()).unwrap();

    let endpoints = vec![
        endpoint("ep-a", "model-a", None),
        endpoint("ep-b", "model-b", None),
        endpoint("orchestrator", "model", None),
    ];
    // Both roles upper-case to SAYA_RUN_EP_REVIEW_1: one env name would
    // carry two endpoints' keys.
    let bindings = EndpointBindings::new([("review-1", "ep-a"), ("review_1", "ep-b")]).unwrap();

    let refused = write_run_configs(&run, &endpoints, &bindings, &[]);
    match refused {
        Err(RunConfigError::DuplicateEnvBinding { var }) => {
            assert_eq!(var, "SAYA_RUN_EP_REVIEW_1");
        }
        other => panic!("expected DuplicateEnvBinding, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(runs_root);
}
