#[test]
fn the_runner_rows_carry_three_distinct_labels_and_their_own_values() {
    let runtime = runtime_from("", "", CliOverrides::default(), BTreeMap::new(), None);
    let explanation = explain(&runtime);
    let allow = entry(&explanation, "jobs.runner.allow");
    let program_dir = entry(&explanation, "jobs.runner.program_dir");
    let timeout = entry(&explanation, "jobs.runner.timeout_seconds");
    assert_eq!(
        [&allow.key, &program_dir.key, &timeout.key],
        [
            "jobs.runner.allow",
            "jobs.runner.program_dir",
            "jobs.runner.timeout_seconds"
        ],
        "three unique dotted keys, never the bare grouped key"
    );
    assert!(!explanation.settings.iter().any(|s| s.key == "jobs.runner"));
    assert_eq!(allow.value, serde_json::json!([]));
    assert_eq!(program_dir.value, serde_json::Value::Null);
    assert_eq!(timeout.value, serde_json::json!(300));
    for setting in [&allow, &program_dir, &timeout] {
        assert_eq!(setting.layer, SettingSource::Default, "{:?}", setting.key);
        assert_eq!(setting.source, "built-in default", "{:?}", setting.key);
    }
    // The origin is still recorded — and looked up — under the grouped
    // `jobs.runner` key the resolver records; only the displayed labels
    // changed.
    assert!(
        runtime.resolved.provenance.get("jobs.runner").is_some(),
        "the grouped origin must stay recorded under jobs.runner"
    );
}

#[test]
fn a_user_set_runner_value_reports_its_real_origin_on_each_runner_row() {
    let runtime = runtime_from(
        "[jobs.runner]\ntimeout_seconds = 120\n",
        "",
        CliOverrides::default(),
        BTreeMap::new(),
        None,
    );
    let explanation = explain(&runtime);
    for key in [
        "jobs.runner.allow",
        "jobs.runner.program_dir",
        "jobs.runner.timeout_seconds",
    ] {
        let setting = entry(&explanation, key);
        assert_eq!(setting.layer, SettingSource::UserFile, "{key}");
        assert_eq!(setting.source, "user config file", "{key}");
        assert_eq!(
            setting.file.as_deref(),
            Some(
                crate::config::sources::user_config_dir()
                    .join("config.toml")
                    .to_str()
                    .unwrap()
            ),
            "{key}"
        );
    }
    let timeout = entry(&explanation, "jobs.runner.timeout_seconds");
    assert_eq!(timeout.value, serde_json::json!(120));
    // Grouped resolver behavior: the recorder records the section's origin
    // as a whole, so every runner row carries it even for a sub-key that
    // was not set — the display must not claim per-field precision the
    // resolver never recorded.
    let allow = entry(&explanation, "jobs.runner.allow");
    assert_eq!(allow.value, serde_json::json!([]));
    assert_eq!(allow.layer, SettingSource::UserFile);
}

#[test]
fn a_project_set_runner_value_reports_the_project_file_on_each_runner_row() {
    let project = PathBuf::from("/saya-test/project/.saya/config.toml");
    let runtime = runtime_from(
        "",
        "[jobs.runner]\nprogram_dir = '/saya-test/runner-programs'\n",
        CliOverrides::default(),
        BTreeMap::new(),
        Some(project.clone()),
    );
    let explanation = explain(&runtime);
    for key in [
        "jobs.runner.allow",
        "jobs.runner.program_dir",
        "jobs.runner.timeout_seconds",
    ] {
        let setting = entry(&explanation, key);
        assert_eq!(setting.layer, SettingSource::ProjectFile, "{key}");
        assert_eq!(setting.source, "project config file", "{key}");
        assert_eq!(
            setting.file.as_deref(),
            Some(project.to_str().unwrap()),
            "{key}"
        );
    }
    let program_dir = entry(&explanation, "jobs.runner.program_dir");
    assert_eq!(
        program_dir.value,
        serde_json::json!("/saya-test/runner-programs")
    );
}

#[test]
fn the_json_and_text_renderings_reflect_the_distinct_runner_labels_and_values() {
    let runtime = runtime_from(
        "[jobs.runner]\ntimeout_seconds = 120\n",
        "",
        CliOverrides::default(),
        BTreeMap::new(),
        None,
    );
    let explanation = explain(&runtime);
    let value: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&explanation).unwrap()).unwrap();
    let settings = value["settings"].as_array().unwrap();
    for label in [
        "jobs.runner.allow",
        "jobs.runner.program_dir",
        "jobs.runner.timeout_seconds",
    ] {
        assert!(settings.iter().any(|s| s["key"] == label), "{value}");
    }
    let timeout = settings
        .iter()
        .find(|s| s["key"] == "jobs.runner.timeout_seconds")
        .unwrap();
    assert_eq!(timeout["value"], serde_json::json!(120));
    assert_eq!(timeout["source"], "user config file");
    let rendered = text(&explanation);
    for label in [
        "jobs.runner.allow",
        "jobs.runner.program_dir",
        "jobs.runner.timeout_seconds",
    ] {
        assert!(rendered.contains(label), "{rendered}");
    }
    assert!(rendered.contains("user config file"), "{rendered}");
}
