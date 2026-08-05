#[test]
fn environment_only_named_profiles_are_available_to_connection_commands() {
    let root = std::env::temp_dir().join(format!("saya-cli-env-profile-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let options = saya_cli::GlobalOptions {
        profile: Some("env-only".into()),
        ..Default::default()
    };
    let runtime = saya_cli::load_with_sources(
        &options,
        &root,
        &root,
        std::collections::BTreeMap::from([
            ("SAYA_DB_TYPE".into(), "postgresql".into()),
            ("SAYA_DB_HOST".into(), "127.0.0.1".into()),
            ("SAYA_DB_PORT".into(), "1".into()),
            ("SAYA_DB_NAME".into(), "app".into()),
            ("SAYA_DB_USER".into(), "reader".into()),
        ]),
    )
    .unwrap();
    assert!(runtime.connections.profiles.is_empty());
    assert_eq!(runtime.resolved.profile_name.as_deref(), Some("env-only"));
    assert!(runtime.named_profile("env-only").is_ok());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn connection_subcommands_accept_environment_only_named_profiles() {
    let root = std::env::temp_dir().join(format!("saya-cli-env-command-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for command in [
        ["connection", "test", "env-only"],
        ["connection", "schema", "env-only"],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
            .args(["--non-interactive", "--format", "json"])
            .args(command)
            .env("SAYA_CONFIG_HOME", &root)
            .env("SAYA_PROFILE", "env-only")
            .env("SAYA_DB_TYPE", "postgresql")
            .env("SAYA_DB_HOST", "127.0.0.1")
            .env("SAYA_DB_PORT", "1")
            .env("SAYA_DB_NAME", "app")
            .env("SAYA_DB_USER", "reader")
            .env("SAYA_QUERY_TIMEOUT_SECONDS", "1")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("\"event\":\"error\""),
            "command: {command:?}; stderr: {stderr}"
        );
        assert!(!stderr.contains("profile \"env-only\" was not found"));
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mysql_environment_profile_reaches_generic_connector_path() {
    let root = std::env::temp_dir().join(format!("saya-cli-mysql-command-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "json",
            "connection",
            "test",
            "env-only",
        ])
        .env("SAYA_CONFIG_HOME", &root)
        .env("SAYA_PROFILE", "env-only")
        .env("SAYA_DB_TYPE", "mysql")
        .env("SAYA_DB_HOST", "127.0.0.1")
        .env("SAYA_DB_PORT", "1")
        .env("SAYA_DB_NAME", "app")
        .env("SAYA_DB_USER", "reader")
        .env("SAYA_DB_SSLMODE", "disable")
        .env("SAYA_QUERY_TIMEOUT_SECONDS", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("\"event\":\"error\""));
    assert!(!stderr.contains("not_implemented"));
    assert!(!stderr.contains("connector is not implemented"));
    assert!(output.stdout.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn snowflake_environment_profiles_reach_connector_and_externalbrowser_fails_before_network() {
    let root = std::env::temp_dir().join(format!("saya-cli-snowflake-env-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for (auth, secret_name, secret_value) in [
        ("keypair", "SAYA_DB_PRIVATE_KEY", "invalid-key-sentinel"),
        ("userpass", "SAYA_DB_PASSWORD", "password-sentinel"),
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
            .args([
                "--non-interactive",
                "--format",
                "json",
                "connection",
                "test",
                "env-only",
            ])
            .env("SAYA_CONFIG_HOME", &root)
            .env("SAYA_PROFILE", "env-only")
            .env("SAYA_DB_TYPE", "snowflake")
            .env("SAYA_DB_ACCOUNT", "bad/account")
            .env("SAYA_DB_USER", "reader")
            .env("SAYA_DB_AUTH_TYPE", auth)
            .env(secret_name, secret_value)
            .env("SAYA_QUERY_TIMEOUT_SECONDS", "1")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3), "{auth}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("\"event\":\"error\""));
        assert!(!stderr.contains(secret_value));
        assert!(output.stdout.is_empty());
    }

    let browser = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "json",
            "connection",
            "test",
            "env-only",
        ])
        .env("SAYA_CONFIG_HOME", &root)
        .env("SAYA_PROFILE", "env-only")
        .env("SAYA_DB_TYPE", "snowflake")
        .env("SAYA_DB_ACCOUNT", "acct")
        .env("SAYA_DB_USER", "reader")
        .env("SAYA_DB_AUTH_TYPE", "externalbrowser")
        .output()
        .unwrap();
    assert_eq!(browser.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&browser.stderr);
    assert!(stderr.contains("interactive mode"));
    assert!(browser.stdout.is_empty());

    let piped = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(["--format", "json", "connection", "test", "env-only"])
        .env("SAYA_CONFIG_HOME", &root)
        .env("SAYA_PROFILE", "env-only")
        .env("SAYA_DB_TYPE", "snowflake")
        .env("SAYA_DB_ACCOUNT", "acct")
        .env("SAYA_DB_USER", "reader")
        .env("SAYA_DB_AUTH_TYPE", "externalbrowser")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let piped_output = piped.wait_with_output().unwrap();
    assert_eq!(piped_output.status.code(), Some(3));
    let piped_text = format!(
        "{}{}",
        String::from_utf8_lossy(&piped_output.stdout),
        String::from_utf8_lossy(&piped_output.stderr)
    );
    assert!(piped_text.contains("\"event\":\"error\""), "{piped_text}");
    assert!(piped_text.contains("interactive mode"), "{piped_text}");
    assert!(piped_output.stdout.is_empty(), "{piped_text}");
    let _ = std::fs::remove_dir_all(root);
}
