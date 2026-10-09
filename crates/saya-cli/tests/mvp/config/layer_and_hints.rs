/// A user-file model is attributed to the user config file, and the path is
/// the one this run actually read (SAYA_CONFIG_HOME points into the root).
#[test]
fn config_show_explain_names_the_user_file_layer_for_a_user_setting() {
    let root = test_root("explain-user-file");
    fs::create_dir_all(root.join("user-config/saya")).unwrap();
    fs::write(
        root.join("user-config/saya/config.toml"),
        "[ai]\nmodel = 'x'\n",
    )
    .unwrap();
    let output = saya_isolated(&root, &["config", "show", "--explain"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let model_line = stdout
        .lines()
        .find(|line| line.starts_with("ai.model"))
        .unwrap_or_else(|| panic!("no ai.model line: {stdout}"));
    let expected = format!(
        "{:<38} {}  ← user config file ({})",
        "ai.model",
        "x",
        root.join("user-config/saya/config.toml").display()
    );
    assert_eq!(model_line, expected, "{stdout}");
    let _ = fs::remove_dir_all(root);
}

/// `SAYA_MODEL=y` wins over nothing else, so the model line names the
/// environment variable — the variable NAME, never a file.
#[test]
fn config_show_explain_names_the_environment_layer_for_an_env_override() {
    let root = test_root("explain-env");
    let output = saya_isolated_env(
        &root,
        &["config", "show", "--explain"],
        &[("SAYA_MODEL", "y")],
    );
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let model_line = stdout
        .lines()
        .find(|line| line.starts_with("ai.model"))
        .unwrap_or_else(|| panic!("no ai.model line: {stdout}"));
    assert!(
        model_line.ends_with("← environment (SAYA_MODEL)"),
        "{model_line}"
    );
    let _ = fs::remove_dir_all(root);
}

/// JSON format: exactly one JSON object with `settings` and
/// `ignored_project_overrides`; an ignored project `ai.base_url` carries the
/// pinned reason and the effective value stays the user's.
#[test]
fn config_show_explain_json_emits_one_object_with_settings_and_ignored_overrides() {
    let root = test_root("explain-json");
    fs::create_dir_all(root.join("user-config/saya")).unwrap();
    fs::write(
        root.join("user-config/saya/config.toml"),
        "[ai]\nbase_url = 'https://user.example/v1'\n",
    )
    .unwrap();
    fs::create_dir_all(root.join(".saya")).unwrap();
    fs::write(
        root.join(".saya/config.toml"),
        "[ai]\nbase_url = 'https://project.example/v1'\n",
    )
    .unwrap();
    let output = saya_isolated(&root, &["--format", "json", "config", "show", "--explain"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "one JSON object on stdout: {stdout:?}");
    let value: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let settings = value["settings"].as_array().expect("settings array");
    let base_url = settings
        .iter()
        .find(|setting| setting["key"] == "ai.base_url")
        .unwrap_or_else(|| panic!("no ai.base_url setting: {value}"));
    let winning = base_url["value"].as_str().unwrap();
    assert!(winning.contains("user.example"), "{base_url}");
    assert!(!winning.contains("project.example"), "{base_url}");
    let ignored = value["ignored_project_overrides"]
        .as_array()
        .expect("ignored_project_overrides array");
    assert!(
        ignored.iter().any(|entry| {
            entry["key"] == "ai.base_url"
                && entry["reason"]
                    == "the project file cannot set ai.base_url: it is security-critical; \
                        set it in your user config file or pass --trust-project-config"
        }),
        "{value}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Plain `config show` keeps the old diagnostics shape: it parses as JSON
/// with `provider` and no `settings` key — the explain surface is opt-in.
#[test]
fn plain_config_show_keeps_the_old_diagnostics_shape() {
    let root = test_root("explain-plain-show-compat");
    let output = saya_isolated(&root, &["--format", "json", "config", "show"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&output.stdout).trim()).unwrap();
    assert!(value.get("provider").is_some(), "{value}");
    assert!(
        value.get("settings").is_none(),
        "plain config show must not grow an explanation: {value}"
    );
    let _ = fs::remove_dir_all(root);
}

/// A missing cloud credentials check prints a `fix:` line naming the setting
/// and the user config file to edit.
#[test]
fn doctor_task_ai_prints_a_fix_hint_when_the_key_is_missing() {
    let root = test_root("doctor-task-ai-hint");
    fs::create_dir_all(root.join("user-config/saya")).unwrap();
    fs::write(
        root.join("user-config/saya/config.toml"),
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\n",
    )
    .unwrap();
    let output = saya_isolated(&root, &["config", "doctor", "--task", "ai"]);
    assert_eq!(output.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected = format!(
        "Set [ai] api_key = {{ env = \"NAME\" }} in {} (or export SAYA_AI_API_KEY).",
        root.join("user-config/saya/config.toml").display()
    );
    assert!(stdout.contains("fix:"), "{stdout}");
    assert!(stdout.contains(&expected), "{stdout}");
    let _ = fs::remove_dir_all(root);
}

/// NDJSON: the failing check carries a string `hint`; checks that pass carry
/// no `hint` key at all (absent, not null).
#[test]
fn doctor_task_ai_ndjson_carries_a_hint_only_where_there_is_one() {
    let root = test_root("doctor-task-ai-hint-ndjson");
    fs::create_dir_all(root.join("user-config/saya")).unwrap();
    fs::write(
        root.join("user-config/saya/config.toml"),
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\n",
    )
    .unwrap();
    let output = saya_isolated(
        &root,
        &["--format", "ndjson", "config", "doctor", "--task", "ai"],
    );
    assert_eq!(output.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "one ndjson event: {stdout:?}");
    let value: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let checks = value["checks"].as_array().unwrap();
    let credentials = checks
        .iter()
        .find(|check| check["name"] == "credentials")
        .unwrap();
    assert_eq!(credentials["status"], "fail");
    assert!(credentials["hint"].is_string(), "{credentials}");
    for check in checks {
        if check["status"] != "fail" && check["status"] != "warn" {
            assert!(
                check.get("hint").is_none(),
                "a passing check must not carry a hint: {check}"
            );
        }
    }
    let _ = fs::remove_dir_all(root);
}
