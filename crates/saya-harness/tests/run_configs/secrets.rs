//! The generated run configs bind a key reference, never a resolved
//! value: every file under the run dir is byte-scanned for the planted
//! secret, and the original reference is replaced by the run-scoped one.

use std::path::{Path, PathBuf};

use super::{corpus, endpoint, temp_root};
use saya_harness::endpoints::write_run_configs;
use saya_harness::run_dir::RunDir;
use saya_types::{EndpointBindings, RunId, SecretRef};

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
