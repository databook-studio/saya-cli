use clap::{CommandFactory, Parser};
use saya_cli::{
    Cli, Command, FormatArg, RenderFormat, SessionAction, SessionState, SlashCommand,
    TerminalEvent, approval_name, parse_slash_command, render_event, resolve_session_dir,
};

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
        SessionAction::NotImplemented(_)
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
fn scripted_repl_persists_and_continues_a_redacted_session() {
    use std::io::Write;
    let root = std::env::temp_dir().join(format!("saya-cli-repl-{}", std::process::id()));
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
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
fn automation_not_implemented_uses_stable_nonzero_exit_codes_and_envelopes() {
    let root = std::env::temp_dir().join(format!("saya-cli-unavailable-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        "[profiles.analytics]\ntype = 'postgresql'\nhost = 'db.test'\ndatabase = 'app'\nuser = 'reader'\n\n[profiles.local]\ntype = 'duckdb'\npath = 'data.duckdb'\n",
    )
    .unwrap();
    let cases = [
        (
            &[
                "--connections",
                connections.to_str().unwrap(),
                "connection",
                "test",
                "local",
            ][..],
            3,
            "duckdb connector",
        ),
        (
            &[
                "--connections",
                connections.to_str().unwrap(),
                "--profile",
                "local",
                "query",
                "--sql",
                "select 1",
            ][..],
            4,
            "duckdb connector",
        ),
        (&["ask", "show revenue"][..], 5, "AI/database execution"),
    ];
    for (args, code, expected) in cases {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_saya"));
        if expected == "AI/database execution" {
            command.env("SAYA_PROVIDER_BASE_URL", "http://127.0.0.1:1");
        }
        let output = command
            .args(["--non-interactive", "--format", "json"])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code), "args: {args:?}");
        if expected == "AI/database execution" {
            assert!(String::from_utf8_lossy(&output.stderr).contains("\"event\":\"error\""));
            assert!(output.stdout.is_empty());
        } else {
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("\"event\":\"not_implemented\"")
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains(expected));
            assert!(output.stderr.is_empty());
        }
    }
    std::fs::remove_dir_all(root).unwrap();
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
        let body = r#"{"choices":[{"message":{"content":"answer from mock"}}]}"#;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let root = std::env::temp_dir().join(format!("saya-cli-ask-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "json",
            "ask",
            "show revenue",
        ])
        .env("SAYA_CONFIG_HOME", &root)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .output()
        .unwrap();
    handle.join().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("answer from mock"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("mock-secret"));
    assert!(output.stderr.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}
