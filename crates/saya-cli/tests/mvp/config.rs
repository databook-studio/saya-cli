use crate::common::*;
use std::fs;

#[test]
fn config_init_creates_parseable_templates_with_stable_output() {
    for (format, expected) in [
        (
            "text",
            "Created .saya/config.toml and .saya/connections.toml\n",
        ),
        (
            "json",
            "{\"event\":\"result\",\"message\":\"Created .saya/config.toml and .saya/connections.toml\"}\n",
        ),
        (
            "ndjson",
            "{\"event\":\"result\",\"message\":\"Created .saya/config.toml and .saya/connections.toml\"}\n",
        ),
    ] {
        let root = test_root(&format!("init-{format}"));
        let output = saya_process(&root, &["--format", format, "config", "init"]);
        assert!(output.status.success(), "stderr: {}", stderr(&output));
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
        assert!(output.stderr.is_empty());

        let config = fs::read_to_string(root.join(".saya/config.toml")).unwrap();
        let connections = fs::read_to_string(root.join(".saya/connections.toml")).unwrap();
        saya_config::ConfigFile::from_toml(&config).unwrap();
        saya_config::ConnectionsFile::from_toml(&connections).unwrap();
        assert!(connections.contains("{ env = \"SAYA_ANALYTICS_PASSWORD\" }"));
        assert!(!config.contains("password = \""));
        assert!(saya_process(&root, &["config", "doctor"]).status.success());
        assert!(
            saya_process(&root, &["connection", "list"])
                .status
                .success()
        );
        assert!(
            saya_process(&root, &["config", "show", "--resolved", "--redacted"])
                .status
                .success()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(root.join(".saya"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            for name in ["config.toml", "connections.toml"] {
                assert_eq!(
                    fs::metadata(root.join(".saya").join(name))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn config_init_refuses_overwrite_and_rolls_back_partial_creation() {
    let root = test_root("init-no-overwrite");
    let first = saya_process(&root, &["config", "init"]);
    assert!(first.status.success());
    let config_before = fs::read_to_string(root.join(".saya/config.toml")).unwrap();
    let second = saya_process(&root, &["config", "init"]);
    assert_eq!(second.status.code(), Some(2));
    assert!(stderr(&second).contains("already exists"));
    assert_eq!(
        fs::read_to_string(root.join(".saya/config.toml")).unwrap(),
        config_before
    );
    fs::remove_dir_all(&root).unwrap();

    let root = test_root("init-rollback");
    fs::create_dir_all(root.join(".saya")).unwrap();
    fs::create_dir(root.join(".saya/connections.toml")).unwrap();
    let output = saya_process(&root, &["config", "init"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("connections.toml"));
    assert!(!root.join(".saya/config.toml").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn config_init_failures_use_stable_error_envelopes_without_path_leaks() {
    for (format, expected) in [
        ("text", ".saya/connections.toml already exists\n"),
        (
            "json",
            "{\"event\":\"error\",\"message\":\".saya/connections.toml already exists\"}\n",
        ),
        (
            "ndjson",
            "{\"event\":\"error\",\"message\":\".saya/connections.toml already exists\"}\n",
        ),
    ] {
        let root = test_root(&format!("init-failure-{format}"));
        fs::create_dir(root.join(".saya")).unwrap();
        fs::create_dir(root.join(".saya/connections.toml")).unwrap();
        let output = saya_process(&root, &["--format", format, "config", "init"]);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8_lossy(&output.stderr), expected);
        assert!(!stderr(&output).contains(&root.display().to_string()));
        assert!(!root.join(".saya/config.toml").exists());
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn discovery_prefers_project_over_user_and_explicit_env_file_is_opt_in() {
    let root = std::env::temp_dir().join(format!("saya-cli-discovery-{}", std::process::id()));
    let user = root.join("user");
    let project = root.join("project");
    std::fs::create_dir_all(project.join(".saya")).unwrap();
    std::fs::create_dir_all(&user).unwrap();
    std::fs::write(user.join("config.toml"), "[ai]\nmodel = 'user'\n").unwrap();
    std::fs::write(
        project.join(".saya/config.toml"),
        "[ai]\nmodel = 'project'\n",
    )
    .unwrap();
    let options = saya_cli::GlobalOptions::default();
    let loaded =
        saya_cli::load_with_sources(&options, &project, &user, std::collections::BTreeMap::new())
            .unwrap();
    assert_eq!(loaded.resolved.ai.model, "project");
    std::fs::write(project.join(".env"), "SAYA_AI_MODEL = 'ignored'\n").unwrap();
    let without_opt_in =
        saya_cli::load_with_sources(&options, &project, &user, std::collections::BTreeMap::new())
            .unwrap();
    assert_eq!(without_opt_in.resolved.ai.model, "project");
    let env_file = project.join(".env.saya");
    std::fs::write(&env_file, "SAYA_AI_MODEL=explicit-env\n").unwrap();
    let explicit = saya_cli::GlobalOptions {
        env_file: Some(env_file),
        ..Default::default()
    };
    let with_opt_in = saya_cli::load_with_sources(
        &explicit,
        &project,
        &user,
        std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(with_opt_in.resolved.ai.model, "explicit-env");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn runtime_debug_redacts_merged_environment_values_and_keys() {
    let root = std::env::temp_dir().join(format!("saya-cli-debug-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let sentinel_key = "SAYA_TEST_SECRET_SENTINEL";
    let sentinel_value = "never-print-this-secret";
    let runtime = saya_cli::load_with_sources(
        &saya_cli::GlobalOptions::default(),
        &root,
        &root,
        std::collections::BTreeMap::from([(sentinel_key.into(), sentinel_value.into())]),
    )
    .unwrap();
    let diagnostic = format!("{runtime:?}");
    assert!(!diagnostic.contains(sentinel_key));
    assert!(!diagnostic.contains(sentinel_value));
    std::fs::remove_dir_all(root).unwrap();
}
