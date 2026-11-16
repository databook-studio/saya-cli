#[test]
fn a_user_file_model_is_attributed_to_the_user_config_file() {
    let runtime = runtime_from(
        "[ai]\nmodel = 'user-model'\n",
        "",
        CliOverrides::default(),
        BTreeMap::new(),
        None,
    );
    let explanation = explain(&runtime);
    let model = entry(&explanation, "ai.model");
    assert_eq!(model.layer, SettingSource::UserFile);
    assert_eq!(model.source, "user config file");
    assert_eq!(
        model.file.as_deref(),
        Some(
            crate::config::sources::user_config_dir()
                .join("config.toml")
                .to_str()
                .unwrap()
        )
    );
    assert_eq!(model.value, serde_json::json!("user-model"));
}

#[test]
fn a_project_file_model_is_attributed_to_the_project_config_file() {
    let project = PathBuf::from("/saya-test/project/.saya/config.toml");
    let runtime = runtime_from(
        "",
        "[ai]\nmodel = 'project-model'\n",
        CliOverrides::default(),
        BTreeMap::new(),
        Some(project.clone()),
    );
    let explanation = explain(&runtime);
    let model = entry(&explanation, "ai.model");
    assert_eq!(model.layer, SettingSource::ProjectFile);
    assert_eq!(model.source, "project config file");
    assert_eq!(model.file.as_deref(), Some(project.to_str().unwrap()));
    assert_eq!(model.value, serde_json::json!("project-model"));
}

// --- env and CLI sources name the variable or flag, never a file ---

#[test]
fn an_env_var_source_names_the_variable_and_never_a_file() {
    let env = BTreeMap::from([("SAYA_MODEL".to_string(), "env-model".to_string())]);
    let runtime = runtime_from("", "", CliOverrides::default(), env, None);
    let explanation = explain(&runtime);
    let model = entry(&explanation, "ai.model");
    assert_eq!(
        model.layer,
        SettingSource::Env {
            var: "SAYA_MODEL".into()
        }
    );
    assert_eq!(model.source, "environment (SAYA_MODEL)");
    assert_eq!(model.file, None);
    assert_eq!(model.value, serde_json::json!("env-model"));
}

#[test]
fn a_cli_flag_source_names_the_flag_and_never_a_file() {
    let cli = CliOverrides {
        model: Some("cli-model".into()),
        ..Default::default()
    };
    let runtime = runtime_from("", "", cli, BTreeMap::new(), None);
    let explanation = explain(&runtime);
    let model = entry(&explanation, "ai.model");
    assert_eq!(
        model.layer,
        SettingSource::Cli {
            flag: "--model".into()
        }
    );
    assert_eq!(model.source, "command line (--model)");
    assert_eq!(model.file, None);
}

// --- the ignored project overrides carry the pinned reason ---

#[test]
fn an_ignored_project_override_is_listed_with_its_reason_and_the_users_value_wins() {
    let runtime = runtime_from(
        "[ai]\nbase_url = 'https://user.example/v1'\n",
        "[ai]\nbase_url = 'https://project.example/v1'\n",
        CliOverrides::default(),
        BTreeMap::new(),
        Some(PathBuf::from("/saya-test/project/.saya/config.toml")),
    );
    let explanation = explain(&runtime);
    let ignored = &explanation.ignored_project_overrides;
    assert_eq!(ignored.len(), 1, "{explanation:?}");
    assert_eq!(ignored[0].key, "ai.base_url");
    assert_eq!(
        ignored[0].reason,
        "the project file cannot set ai.base_url: it is security-critical; \
         set it in your user config file or pass --trust-project-config"
    );
    let base_url = entry(&explanation, "ai.base_url");
    assert_eq!(base_url.layer, SettingSource::UserFile);
    let value = base_url.value.as_str().unwrap();
    assert!(value.contains("user.example"), "{value}");
    assert!(!value.contains("project.example"), "{value}");
}
