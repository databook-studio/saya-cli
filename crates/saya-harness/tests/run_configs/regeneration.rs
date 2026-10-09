//! Regeneration rewrites the configs in place: a second pass over the same
//! run does not accumulate endpoint blocks.

use super::{corpus, endpoint, temp_root};
use saya_harness::endpoints::write_run_configs;
use saya_harness::run_dir::RunDir;
use saya_types::{EndpointBindings, RunId};

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
