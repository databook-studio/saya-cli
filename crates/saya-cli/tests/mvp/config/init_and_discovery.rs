#[test]
fn config_init_creates_parseable_templates_with_stable_output() {
    // `config init` now writes the user layer (root/user-config/saya under
    // the test's SAYA_CONFIG_HOME), not .saya/. The success message names that
    // directory, so its exact bytes vary per run; the stable prefix and the
    // parseable/permissions/follow-up checks are what the invariant guards.
    let user_dir = |root: &std::path::Path| root.join("user-config/saya");
    for format in ["text", "json", "ndjson"] {
        let root = test_root(&format!("init-{format}"));
        let output = saya_process(&root, &["--format", format, "config", "init"]);
        assert!(output.status.success(), "stderr: {}", stderr(&output));
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Created config.toml and connections.toml in"),
            "stable message prefix: {stdout}"
        );
        assert!(output.stderr.is_empty());

        let config = fs::read_to_string(user_dir(&root).join("config.toml")).unwrap();
        let connections = fs::read_to_string(user_dir(&root).join("connections.toml")).unwrap();
        saya_config::ConfigFile::from_toml(&config).unwrap();
        saya_config::ConnectionsFile::from_toml(&connections).unwrap();
        assert!(connections.contains("{ env = \"SAYA_ANALYTICS_PASSWORD\" }"));
        assert!(!config.contains("password = \""));

        // The rest of the CLI loads the created config. `connection list` and
        // `config show` do not need the secret. `config doctor` now exits
        // non-zero when the secret is unresolved, so resolve it to keep
        // this a "doctor reports a working post-init setup" check.
        assert!(
            saya_process(&root, &["connection", "list"])
                .status
                .success()
        );
        assert!(saya_process(&root, &["config", "show"]).status.success());
        let doctor = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
            .args(["config", "doctor"])
            .current_dir(&root)
            .env("SAYA_CONFIG_HOME", root.join("user-config"))
            .env("SAYA_ANALYTICS_PASSWORD", "a-resolved-secret")
            .output()
            .unwrap();
        assert!(doctor.status.success(), "stderr: {}", stderr(&doctor));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(user_dir(&root)).unwrap().permissions().mode() & 0o777,
                0o700
            );
            for name in ["config.toml", "connections.toml"] {
                assert_eq!(
                    fs::metadata(user_dir(&root).join(name))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
        }
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn config_init_refuses_overwrite_and_rolls_back_partial_creation() {
    let user_dir = |root: &std::path::Path| root.join("user-config/saya");
    let root = test_root("init-no-overwrite");
    let first = saya_process(&root, &["config", "init"]);
    assert!(first.status.success());
    let config_before = fs::read_to_string(user_dir(&root).join("config.toml")).unwrap();
    let second = saya_process(&root, &["config", "init"]);
    assert_eq!(second.status.code(), Some(2));
    assert!(stderr(&second).contains("already exists"));
    assert_eq!(
        fs::read_to_string(user_dir(&root).join("config.toml")).unwrap(),
        config_before
    );
    let _ = fs::remove_dir_all(&root);

    let root = test_root("init-rollback");
    fs::create_dir_all(user_dir(&root)).unwrap();
    fs::create_dir(user_dir(&root).join("connections.toml")).unwrap();
    let output = saya_process(&root, &["config", "init"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("connections.toml"));
    assert!(!user_dir(&root).join("config.toml").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn config_init_failures_use_stable_error_envelopes_without_path_leaks() {
    let user_dir = |root: &std::path::Path| root.join("user-config/saya");
    for (format, expected) in [
        ("text", "connections.toml already exists\n"),
        (
            "json",
            "{\"event\":\"error\",\"message\":\"connections.toml already exists\"}\n",
        ),
        (
            "ndjson",
            "{\"event\":\"error\",\"message\":\"connections.toml already exists\"}\n",
        ),
    ] {
        let root = test_root(&format!("init-failure-{format}"));
        fs::create_dir_all(user_dir(&root)).unwrap();
        fs::create_dir(user_dir(&root).join("connections.toml")).unwrap();
        let output = saya_process(&root, &["--format", format, "config", "init"]);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8_lossy(&output.stderr), expected);
        assert!(!stderr(&output).contains(&root.display().to_string()));
        assert!(!user_dir(&root).join("config.toml").exists());
        let _ = fs::remove_dir_all(root);
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
    let _ = std::fs::remove_dir_all(root);
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
    let _ = std::fs::remove_dir_all(root);
}
