//! The generated connections: the run corpus is held and no production
//! profile is present, and every corpus path must stay inside the run
//! directory.

use super::{corpus, endpoint, temp_root};
use saya_harness::endpoints::{RunConfigError, write_run_configs};
use saya_harness::run_dir::RunDir;
use saya_types::{EndpointBindings, RunId};

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
