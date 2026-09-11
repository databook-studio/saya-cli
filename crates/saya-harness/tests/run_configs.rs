use std::path::{Path, PathBuf};

use saya_harness::endpoints::{CorpusProfile, EndpointSpec, RunConfigError, write_run_configs};
use saya_harness::run_dir::RunDir;
use saya_types::{EndpointBindings, RunId, SecretRef};

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "saya-harness-run-configs-{label}-{}",
        std::process::id()
    ))
}

fn endpoint(name: &str, model: &str, api_key: Option<SecretRef>) -> EndpointSpec {
    EndpointSpec {
        name: name.to_string(),
        provider: "anthropic".to_string(),
        model: model.to_string(),
        base_url: Some("https://api.test".to_string()),
        api_key,
    }
}

fn corpus(name: &str, path: &str) -> CorpusProfile {
    CorpusProfile {
        name: name.to_string(),
        path: path.to_string(),
    }
}

/// Every regular file under `root`, at any depth.
fn run_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}

/// A resolved value a future writer could only produce by resolving the
/// endpoint's secret reference. Distinctive enough to byte-scan for.
const PLANTED_VAR: &str = "SAYA_HARNESS_PLANTED_KEY";
const PLANTED_VALUE: &str = "planted-9f3a-o4fJq9xZ-secret-value";

#[test]
fn generated_run_configs_bind_a_key_reference_and_never_a_resolved_value() {
    let runs_root = temp_root("byte-scan");
    let run = RunDir::create(&runs_root, &RunId::parse("scan-1").unwrap()).unwrap();
    unsafe { std::env::set_var(PLANTED_VAR, PLANTED_VALUE) };

    let endpoints = vec![endpoint(
        "primary",
        "claude-test-model",
        Some(SecretRef::Env {
            env: PLANTED_VAR.to_string(),
        }),
    )];
    let bindings = EndpointBindings::new([("orchestrator", "primary")]).unwrap();
    let corpus = vec![corpus("corp", "workspace/corpus/corp.db")];

    write_run_configs(&run, &endpoints, &bindings, &corpus).unwrap();

    let files = run_files(run.root());
    assert!(
        files.len() >= 2,
        "the two config files must exist: {files:?}"
    );
    let needle = PLANTED_VALUE.as_bytes();
    for file in &files {
        let bytes = std::fs::read(file).unwrap();
        assert!(
            !bytes.windows(needle.len()).any(|window| window == needle),
            "a resolved secret value reached disk: {}",
            file.display()
        );
    }

    let config = std::fs::read_to_string(run.state().join("config/config.toml")).unwrap();
    assert!(config.contains("api_key = { env = \"SAYA_RUN_EP_ORCHESTRATOR\" }"));
    // The original reference is replaced by the run-scoped one, not echoed.
    assert!(!config.contains(PLANTED_VAR));

    unsafe { std::env::remove_var(PLANTED_VAR) };
    let _ = std::fs::remove_dir_all(runs_root);
}

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
fn generated_connections_hold_the_run_corpus_and_no_production_profile() {
    let runs_root = temp_root("corpus-only");
    let run = RunDir::create(&runs_root, &RunId::parse("corpus-1").unwrap()).unwrap();

    let endpoints = vec![endpoint("orchestrator", "model", None)];
    let bindings = EndpointBindings::default();
    let corpus = vec![corpus("corp", "workspace/corpus/corp.db")];

    write_run_configs(&run, &endpoints, &bindings, &corpus).unwrap();

    let connections = std::fs::read_to_string(run.state().join("config/connections.toml")).unwrap();
    assert!(connections.contains("[profiles.\"corp\"]"));
    assert!(connections.contains("path = \"workspace/corpus/corp.db\""));
    // Absence by name: a production profile is not denied, it is not there.
    assert!(!connections.contains("analytics"));
    assert!(!connections.contains("snowflake_prod"));

    let _ = std::fs::remove_dir_all(runs_root);
}

#[test]
fn generated_corpus_paths_stay_inside_the_run_directory() {
    let runs_root = temp_root("corpus-paths");
    let run = RunDir::create(&runs_root, &RunId::parse("path-1").unwrap()).unwrap();

    let endpoints = vec![endpoint("orchestrator", "model", None)];
    let bindings = EndpointBindings::default();
    write_run_configs(
        &run,
        &endpoints,
        &bindings,
        &[corpus("corp", "workspace/corpus/corp.db")],
    )
    .unwrap();
    let connections = std::fs::read_to_string(run.state().join("config/connections.toml")).unwrap();
    assert!(connections.contains("path = \"workspace/corpus/corp.db\""));

    for bad in ["", "../escape.db", "/etc/keys.db", "corpus/../../escape.db"] {
        let refused = write_run_configs(&run, &endpoints, &bindings, &[corpus("corp", bad)]);
        match refused {
            Err(RunConfigError::CorpusPathOutsideRun { path }) => assert_eq!(path, bad),
            other => panic!("expected CorpusPathOutsideRun for {bad:?}, got {other:?}"),
        }
    }

    let _ = std::fs::remove_dir_all(runs_root);
}

#[test]
#[cfg(unix)]
fn generated_files_are_private_and_a_loose_mode_is_repaired() {
    use std::os::unix::fs::PermissionsExt;

    let runs_root = temp_root("private-files");
    let run = RunDir::create(&runs_root, &RunId::parse("mode-1").unwrap()).unwrap();

    let endpoints = vec![endpoint("orchestrator", "model", None)];
    let bindings = EndpointBindings::default();
    let corpus = vec![corpus("corp", "workspace/corpus/corp.db")];

    write_run_configs(&run, &endpoints, &bindings, &corpus).unwrap();
    let config_path = run.state().join("config/config.toml");
    let connections_path = run.state().join("config/connections.toml");
    for file in [&config_path, &connections_path] {
        assert_eq!(
            std::fs::metadata(file).unwrap().permissions().mode() & 0o077,
            0,
            "{}",
            file.display()
        );
    }
    let config_dir = run.state().join("config");
    assert_eq!(
        std::fs::metadata(&config_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );

    // A loose file mode does not survive a regeneration.
    let loose = std::fs::Permissions::from_mode(0o644);
    std::fs::set_permissions(&config_path, loose).unwrap();
    write_run_configs(&run, &endpoints, &bindings, &corpus).unwrap();
    assert_eq!(
        std::fs::metadata(&config_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o077,
        0
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

#[test]
fn generating_twice_rewrites_in_place() {
    let runs_root = temp_root("rewrite");
    let run = RunDir::create(&runs_root, &RunId::parse("again-1").unwrap()).unwrap();

    let endpoints = vec![endpoint("orchestrator", "model", None)];
    let bindings = EndpointBindings::default();
    let corpus = vec![corpus("corp", "workspace/corpus/corp.db")];

    write_run_configs(&run, &endpoints, &bindings, &corpus).unwrap();
    write_run_configs(&run, &endpoints, &bindings, &corpus).unwrap();
    let config_path = run.state().join("config/config.toml");
    let config = std::fs::read_to_string(&config_path).unwrap();
    assert_eq!(config.matches("[[endpoint]]").count(), 1);

    let _ = std::fs::remove_dir_all(runs_root);
}
