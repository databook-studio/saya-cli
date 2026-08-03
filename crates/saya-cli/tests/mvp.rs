use clap::{CommandFactory, Parser};
use saya_cli::{
    Cli, Command, FormatArg, RenderFormat, SessionAction, SessionState, SlashCommand,
    TerminalEvent, approval_name, parse_slash_command, render_event, resolve_session_dir,
    resolve_state_db_path,
};
use std::{fs, path::Path, process::Command as ProcessCommand};

#[test]
fn help_exposes_interactive_and_automation_flags() {
    let help = Cli::command().render_help().to_string();
    for flag in [
        "--continue",
        "--resume",
        "--include-profile",
        "--approval-mode",
        "--env-file",
        "--allow-data-sharing",
        "--non-interactive",
    ] {
        assert!(help.contains(flag), "missing {flag}");
    }
}

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

fn test_root(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("saya-cli-{label}-{}", std::process::id()));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(&root).unwrap();
    root
}

fn saya_process(root: &Path, args: &[&str]) -> std::process::Output {
    ProcessCommand::new(env!("CARGO_BIN_EXE_saya"))
        .args(args)
        .current_dir(root)
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .output()
        .unwrap()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn parser_is_public_and_testable() {
    let cli = Cli::try_parse_from([
        "saya",
        "--profile",
        "analytics",
        "--format",
        "json",
        "ask",
        "revenue",
    ])
    .unwrap();
    assert_eq!(cli.options.profile.as_deref(), Some("analytics"));
    assert_eq!(cli.options.format, FormatArg::Json);
    assert!(
        matches!(cli.command, Some(Command::Ask { prompt: Some(value), .. }) if value == "revenue")
    );
}

#[test]
fn slash_commands_parse_and_state_transitions_are_deterministic() {
    let mut state = SessionState::new("one", None, "model-a");
    assert_eq!(
        parse_slash_command("/connect analytics").unwrap(),
        Some(SlashCommand::Connect("analytics".into()))
    );
    let selected = state.apply(
        SlashCommand::Connect("analytics".into()),
        &["analytics".into()],
    );
    assert!(
        matches!(selected, SessionAction::Message(ref message) if message == "Selected profile: analytics")
    );
    assert!(matches!(
        state.apply(
            SlashCommand::Connect("unknown".into()),
            &["analytics".into()]
        ),
        SessionAction::Error(_)
    ));
    assert!(matches!(
        state.apply(
            SlashCommand::Include("unknown".into()),
            &["analytics".into()]
        ),
        SessionAction::Error(_)
    ));
    state.apply(
        SlashCommand::Include("staging".into()),
        &["analytics".into(), "staging".into()],
    );
    state.apply(SlashCommand::Privacy(Some(true)), &[]);
    state.apply(
        parse_slash_command("/approvals never").unwrap().unwrap(),
        &[],
    );
    assert_eq!(state.profile.as_deref(), Some("analytics"));
    assert_eq!(state.included_profiles, vec!["staging"]);
    assert!(state.allow_data_sharing);
    assert_eq!(state.approval_mode, "never");
    assert!(matches!(
        state.apply(SlashCommand::History, &[]),
        SessionAction::History
    ));
    assert!(matches!(
        state.apply(SlashCommand::Schema(false), &[]),
        SessionAction::Schema(false)
    ));
    assert_eq!(state.apply(SlashCommand::Exit, &[]), SessionAction::Exit);
}

#[test]
fn renderer_separates_diagnostics_and_emits_stable_envelopes() {
    let result = render_event(
        &TerminalEvent::Result {
            message: "ok".into(),
        },
        RenderFormat::Json,
    );
    assert_eq!(result.stderr, "");
    assert_eq!(result.stdout, "{\"event\":\"result\",\"message\":\"ok\"}\n");
    let diagnostic = render_event(
        &TerminalEvent::Diagnostic {
            message: "safe".into(),
        },
        RenderFormat::Ndjson,
    );
    assert_eq!(diagnostic.stdout, "");
    assert_eq!(
        diagnostic.stderr,
        "{\"event\":\"diagnostic\",\"message\":\"safe\"}\n"
    );
    let complete = render_event(&TerminalEvent::Complete, RenderFormat::Ndjson);
    assert_eq!(complete.stdout, "{\"event\":\"complete\"}\n");
    assert_eq!(complete.stderr, "");
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
    std::fs::remove_dir_all(root).unwrap();
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
    std::fs::remove_dir_all(root).unwrap();
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
    std::fs::remove_dir_all(root).unwrap();
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
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn scripted_repl_persists_and_continues_a_redacted_session() {
    use std::io::Write;
    let root = std::env::temp_dir().join(format!("saya-cli-repl-{}", std::process::id()));
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(["--format", "ndjson"])
        .env("SAYA_SESSION_DIR", &root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"/help\n/exit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("/help"));
    assert!(std::fs::read_dir(&root).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "json")
    }));
    let resumed = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .env("SAYA_SESSION_DIR", &root)
        .arg("--continue")
        .output()
        .unwrap();
    assert!(resumed.status.success());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn non_interactive_mode_does_not_start_a_prompt() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .arg("--non-interactive")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a subcommand"));
}

#[test]
fn duckdb_commands_have_stable_process_envelopes_and_safety() {
    let root = std::env::temp_dir().join(format!("saya-cli-duckdb-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("data.duckdb");
    let fixture = duckdb::Connection::open(&database).unwrap();
    fixture
        .execute_batch("CREATE TABLE events (id INTEGER, label VARCHAR); INSERT INTO events VALUES (1, 'one'), (2, 'two');")
        .unwrap();
    drop(fixture);
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let config = root.join("config.toml");
    let state = root.join("state.sqlite3");
    std::fs::write(&config, "[run]\nmax_rows = 1\n").unwrap();
    let base = [
        "--non-interactive",
        "--format",
        "json",
        "--config",
        config.to_str().unwrap(),
        "--connections",
        connections.to_str().unwrap(),
    ];
    let test = run_cli(&base, &["connection", "test", "local"], &state);
    assert_eq!(test.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&test.stdout).contains("\"event\":\"result\""));
    let schema = run_cli(&base, &["connection", "schema", "local"], &state);
    assert_eq!(schema.status.code(), Some(0));
    let schema_output = String::from_utf8_lossy(&schema.stdout);
    assert!(schema_output.contains("\"event\":\"schema\""));
    assert!(schema_output.contains("events"));
    let query = run_cli(
        &[
            "--non-interactive",
            "--format",
            "json",
            "--config",
            config.to_str().unwrap(),
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
        ],
        &["query", "--sql", "SELECT id, label FROM events ORDER BY id"],
        &state,
    );
    assert_eq!(query.status.code(), Some(0));
    let query_output = String::from_utf8_lossy(&query.stdout);
    assert!(query_output.contains("\"event\":\"query_result\""));
    assert!(query_output.contains("\"truncated\":true"));
    assert!(query.stderr.is_empty());
    let denied = run_cli(
        &[
            "--non-interactive",
            "--format",
            "json",
            "--config",
            config.to_str().unwrap(),
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
        ],
        &["query", "--sql", "DROP TABLE events"],
        &state,
    );
    assert_eq!(denied.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&denied.stderr).contains("\"event\":\"error\""));
    let missing_read_only = root.join("missing-read-only.toml");
    std::fs::write(
        &missing_read_only,
        format!(
            "[profiles.local]\ntype = 'duckdb'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();
    let missing = run_cli(
        &[
            "--non-interactive",
            "--format",
            "json",
            "--connections",
            missing_read_only.to_str().unwrap(),
        ],
        &["connection", "test", "local"],
        &state,
    );
    assert_eq!(missing.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("read_only explicitly"));
    std::fs::remove_dir_all(root).unwrap();
}

fn run_cli(global: &[&str], command: &[&str], state: &std::path::Path) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(global)
        .args(command)
        .env("SAYA_STATE_DB", state)
        .output()
        .unwrap()
}

#[test]
fn query_results_render_as_text_and_json_without_diagnostics() {
    let result = saya_types::QueryResult {
        columns: vec!["id".into()],
        rows: vec![serde_json::json!([1])],
        row_count: 1,
        truncated: true,
        executed_sql: "SELECT id".into(),
    };
    let text = render_event(
        &TerminalEvent::QueryResult {
            result: result.clone(),
        },
        RenderFormat::Text,
    );
    assert_eq!(text.stdout, "id\n1\n[truncated]\n");
    assert!(text.stderr.is_empty());
    let json = render_event(&TerminalEvent::QueryResult { result }, RenderFormat::Json);
    assert!(json.stdout.contains("\"event\":\"query_result\""));
    assert!(json.stderr.is_empty());
}

#[test]
fn non_interactive_defaults_to_never_approval_but_interactive_defaults_to_ask() {
    let interactive = saya_cli::GlobalOptions::default();
    let non_interactive = saya_cli::GlobalOptions {
        non_interactive: true,
        ..Default::default()
    };
    assert_eq!(approval_name(&interactive).unwrap(), "ask");
    assert_eq!(approval_name(&non_interactive).unwrap(), "never");
    let explicit = saya_cli::GlobalOptions {
        non_interactive: true,
        approval_mode: Some("read-only".into()),
        ..Default::default()
    };
    assert_eq!(approval_name(&explicit).unwrap(), "read-only");
}

#[test]
fn session_path_resolution_is_platform_aware_and_injectable() {
    assert_eq!(
        resolve_session_dir(
            Some("/override"),
            Some("/xdg"),
            Some("/appdata"),
            Some("/home")
        ),
        std::path::PathBuf::from("/override")
    );
    assert_eq!(
        resolve_session_dir(None, Some("/xdg"), Some("/appdata"), Some("/home")),
        std::path::PathBuf::from("/xdg/saya/sessions")
    );
    assert_eq!(
        resolve_session_dir(None, None, Some("/appdata"), Some("/home")),
        std::path::PathBuf::from("/appdata/saya/sessions")
    );
    assert_eq!(
        resolve_session_dir(None, None, None, Some("/home")),
        std::path::PathBuf::from("/home/.local/share/saya/sessions")
    );
}

#[test]
fn state_path_precedence_is_platform_aware_and_injectable() {
    assert_eq!(
        resolve_state_db_path(
            Some("/override.sqlite3"),
            Some("/xdg"),
            Some("/appdata"),
            Some("/home")
        ),
        std::path::PathBuf::from("/override.sqlite3")
    );
    assert_eq!(
        resolve_state_db_path(None, Some("/xdg"), Some("/appdata"), Some("/home")),
        std::path::PathBuf::from("/xdg/saya/state.sqlite3")
    );
    assert_eq!(
        resolve_state_db_path(None, None, Some("/appdata"), Some("/home")),
        std::path::PathBuf::from("/appdata/saya/state.sqlite3")
    );
}

#[test]
fn duckdb_schema_cache_fallback_refresh_and_interactive_schema_are_stable() {
    use saya_store::{AuditOperation, AuditStore, SqliteStateStore};
    use std::io::Write;
    let root = std::env::temp_dir().join(format!("saya-cli-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("state.duckdb");
    duckdb::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE cache_events (id INTEGER);")
        .unwrap();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.analytics]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let state = root.join("private-state.sqlite3");
    let globals = [
        "--non-interactive",
        "--format",
        "json",
        "--connections",
        connections.to_str().unwrap(),
    ];
    let first = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(globals)
        .args(["connection", "schema", "analytics"])
        .env("SAYA_STATE_DB", &state)
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&first.stdout).contains("cache_events"));
    let query = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(globals)
        .args([
            "query",
            "--profile",
            "analytics",
            "--sql",
            "SELECT 'row-secret' AS raw_sql_secret",
        ])
        .env("SAYA_STATE_DB", &state)
        .output()
        .unwrap();
    assert_eq!(query.status.code(), Some(0));
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--format",
            "ndjson",
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "analytics",
        ])
        .env("SAYA_STATE_DB", &state)
        .env("SAYA_SESSION_DIR", root.join("sessions"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"/schema\n/schema refresh\n/exit\n")
        .unwrap();
    let interactive = child.wait_with_output().unwrap();
    assert_eq!(interactive.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&interactive.stdout)
            .matches("\"event\":\"schema\"")
            .count(),
        2
    );
    assert!(interactive.stderr.is_empty());
    std::fs::remove_file(&database).unwrap();
    let cached = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(globals)
        .args(["connection", "schema", "analytics"])
        .env("SAYA_STATE_DB", &state)
        .output()
        .unwrap();
    assert_eq!(cached.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&cached.stdout).contains("\"event\":\"schema\""));
    assert_eq!(
        String::from_utf8_lossy(&cached.stderr),
        "{\"event\":\"diagnostic\",\"message\":\"Using cached schema metadata; it may be stale.\"}\n"
    );
    let refresh = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(globals)
        .args(["connection", "schema", "analytics", "--refresh"])
        .env("SAYA_STATE_DB", &state)
        .output()
        .unwrap();
    assert_eq!(refresh.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&refresh.stderr).contains("\"event\":\"error\""));
    let mut failed_repl = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--format",
            "ndjson",
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "analytics",
        ])
        .env("SAYA_STATE_DB", &state)
        .env("SAYA_SESSION_DIR", root.join("failed-sessions"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    failed_repl
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"/schema refresh\n/exit\n")
        .unwrap();
    let failed_repl = failed_repl.wait_with_output().unwrap();
    assert_eq!(failed_repl.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&failed_repl.stderr).contains("\"event\":\"error\""));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let store = SqliteStateStore::new(&state);
    let audits = runtime.block_on(store.recent_audit(100)).unwrap();
    runtime.block_on(store.close());
    drop(store);
    drop(runtime);
    assert!(
        audits
            .iter()
            .all(|row| row.event.profile_id.starts_with("p-") && row.event.profile_id.len() == 66)
    );
    assert!(
        audits
            .iter()
            .any(|row| row.event.operation == AuditOperation::Query)
    );
    assert!(
        audits
            .iter()
            .filter(|row| row.event.operation == AuditOperation::SchemaRefresh)
            .count()
            >= 4
    );
    let decoded_audit = format!("{audits:?}");
    for sentinel in ["analytics", "state.duckdb", "raw_sql_secret", "row-secret"] {
        assert!(!decoded_audit.contains(sentinel), "audit leaked {sentinel}");
    }
    let mut state_bytes = Vec::new();
    for entry in std::fs::read_dir(&root).unwrap().flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("private-state.sqlite3")
        {
            state_bytes.extend(std::fs::read(entry.path()).unwrap());
        }
    }
    let disk = String::from_utf8_lossy(&state_bytes);
    for sentinel in [
        "analytics",
        "state.duckdb",
        "SELECT 'row-secret'",
        "raw_sql_secret",
        "row-secret",
    ] {
        assert!(!disk.contains(sentinel), "state leaked {sentinel}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn relative_and_existing_parent_state_paths_do_not_change_parent_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("saya-cli-relative-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    let database = root.join("data.duckdb");
    duckdb::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE events (id INTEGER);")
        .unwrap();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.analytics]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let state_paths = [
        std::ffi::OsString::from("relative-state.sqlite3"),
        root.join("shared-state.sqlite3").into_os_string(),
    ];
    for state_path in state_paths {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
            .current_dir(&root)
            .args([
                "--non-interactive",
                "--connections",
                connections.to_str().unwrap(),
                "connection",
                "schema",
                "analytics",
            ])
            .env("SAYA_STATE_DB", state_path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
    }
    assert_eq!(
        std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(root.join("relative-state.sqlite3").exists());
    assert!(root.join("shared-state.sqlite3").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn schema_command_emits_one_persistence_warning_when_all_state_steps_fail() {
    let root = std::env::temp_dir().join(format!("saya-cli-state-warning-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("data.duckdb");
    duckdb::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE events (id INTEGER);")
        .unwrap();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.analytics]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let bad_state = root.join("not-a-database");
    std::fs::create_dir(&bad_state).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "json",
            "--connections",
            connections.to_str().unwrap(),
            "connection",
            "schema",
            "analytics",
            "--refresh",
        ])
        .env("SAYA_STATE_DB", &bad_state)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"event\":\"schema\""));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.matches("Local state store unavailable").count(),
        1,
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn ask_calls_configured_openai_compatible_provider() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 8192];
        let _ = stream.read(&mut request);
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"answer from mock\"}}]}\n\ndata: [DONE]\n\n";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let root = std::env::temp_dir().join(format!("saya-cli-ask-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let provider_state = root.join("provider-only-state.sqlite3");
    let database = root.join("ask.duckdb");
    duckdb::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE revenue (amount INTEGER); INSERT INTO revenue VALUES (7);")
        .unwrap();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "json",
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
            "ask",
            "show revenue",
        ])
        .env("SAYA_CONFIG_HOME", &root)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .env("SAYA_STATE_DB", &provider_state)
        .output()
        .unwrap();
    handle.join().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("answer from mock"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("mock-secret"));
    assert!(output.stderr.is_empty());
    assert!(
        !provider_state.exists(),
        "provider-only ask must not create an agent-query audit"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn idle_eof_exits_the_interactive_process_cleanly() {
    let root = std::env::temp_dir().join(format!("saya-cli-eof-{}", std::process::id()));
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("SAYA_SESSION_DIR", &root)
        .spawn()
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn active_sigint_returns_to_repl_without_persisting_incomplete_turn() {
    use std::{
        ffi::CStr,
        fs::File,
        io::{self, Read, Write},
        net::TcpListener,
        os::fd::{AsRawFd, FromRawFd},
        os::unix::process::CommandExt,
        process::{Child, Command, Stdio},
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    struct ChildGuard(Option<Child>);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                if child.try_wait().unwrap().is_none() {
                    let _ = child.kill();
                }
                let _ = child.wait();
            }
        }
    }

    fn pty_pair() -> (File, File) {
        let master_fd = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
        assert!(
            master_fd >= 0,
            "posix_openpt: {}",
            io::Error::last_os_error()
        );
        assert_eq!(unsafe { libc::grantpt(master_fd) }, 0);
        assert_eq!(unsafe { libc::unlockpt(master_fd) }, 0);
        let slave_name = unsafe { CStr::from_ptr(libc::ptsname(master_fd)) };
        let slave = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(slave_name.to_string_lossy().as_ref())
            .unwrap();
        let master = unsafe { File::from_raw_fd(master_fd) };
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
        (master, slave)
    }

    fn read_until(master: &mut File, needle: &[u8], timeout: Duration) -> Vec<u8> {
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();
        let mut buffer = [0_u8; 4096];
        while Instant::now() < deadline {
            match master.read(&mut buffer) {
                Ok(0) => break,
                Ok(size) => {
                    output.extend_from_slice(&buffer[..size]);
                    if output.windows(needle.len()).any(|window| window == needle) {
                        return output;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                Err(error) => panic!("pty read failed: {error}"),
            }
        }
        panic!(
            "timed out waiting for {:?}; output was {:?}",
            String::from_utf8_lossy(needle),
            String::from_utf8_lossy(&output)
        );
    }

    fn wait_with_deadline(child: &mut Child, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            if child.try_wait().unwrap().is_some() {
                return;
            }
            assert!(Instant::now() < deadline, "interactive child did not exit");
            thread::sleep(Duration::from_millis(20));
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let (accepted_tx, accepted_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(value) => break value,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        };
        accepted_tx.send(()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let _ = stream.read_to_end(&mut request);
    });
    let config_root =
        std::env::temp_dir().join(format!("saya-cli-cancel-config-{}", std::process::id()));
    let session_root =
        std::env::temp_dir().join(format!("saya-cli-cancel-session-{}", std::process::id()));
    let (mut master, slave) = pty_pair();
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave))
        .env_clear()
        .env("SAYA_CONFIG_HOME", &config_root)
        .env("SAYA_SESSION_DIR", &session_root)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret");
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            #[cfg(target_vendor = "apple")]
            let controlling_terminal = libc::TIOCSCTTY as libc::c_ulong;
            #[cfg(not(target_vendor = "apple"))]
            let controlling_terminal = libc::TIOCSCTTY;
            if libc::ioctl(libc::STDIN_FILENO, controlling_terminal, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut guard = ChildGuard(Some(command.spawn().unwrap()));
    let child = guard.0.as_mut().unwrap();
    // Cold debug builds can be slow while workspace tests contend for CPU.
    let prompt = read_until(&mut master, b"saya> ", Duration::from_secs(10));
    assert!(String::from_utf8_lossy(&prompt).contains("saya> "));
    master.write_all(b"incomplete prompt\n").unwrap();
    master.flush().unwrap();
    accepted_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("provider request was not observed");
    assert_eq!(
        unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) },
        0
    );
    let cancelled = read_until(&mut master, b"saya> ", Duration::from_secs(3));
    let cancelled_text = String::from_utf8_lossy(&cancelled);
    assert!(
        cancelled_text.contains("Request cancelled."),
        "{cancelled_text}"
    );
    assert!(cancelled_text.contains("saya> "), "{cancelled_text}");
    master.write_all(b"/exit\n").unwrap();
    master.flush().unwrap();
    drop(master);
    wait_with_deadline(child, Duration::from_secs(3));
    guard.0.take();
    server.join().unwrap();
    let saved = std::fs::read_dir(&session_root)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(value["turns"].as_array().unwrap().len(), 0);
    assert!(!saved.contains("incomplete prompt"));
    assert!(!saved.contains("mock-secret"));
    let _ = std::fs::remove_dir_all(config_root);
    std::fs::remove_dir_all(session_root).unwrap();
}
