//! The generated files are private, the config dir is 0700, and a loose
//! file mode does not survive a regeneration.

use super::{corpus, endpoint, temp_root};
use saya_harness::endpoints::write_run_configs;
use saya_harness::run_dir::RunDir;
use saya_types::{EndpointBindings, RunId};

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
