#[test]
fn a_default_config_reports_every_setting_as_built_in_default() {
    let runtime = runtime_from("", "", CliOverrides::default(), BTreeMap::new(), None);
    let explanation = explain(&runtime);
    assert!(
        explanation.ignored_project_overrides.is_empty(),
        "{explanation:?}"
    );
    assert_eq!(
        explanation.settings.len(),
        saya_config::user_facing_keys().len(),
        "every user-facing setting is explained"
    );
    for (setting, (_, lookup_key)) in explanation
        .settings
        .iter()
        .zip(saya_config::user_facing_keys())
    {
        assert_eq!(setting.layer, SettingSource::Default, "{:?}", setting.key);
        assert_eq!(setting.source, "built-in default", "{:?}", setting.key);
        assert_eq!(setting.file, None, "{:?}", setting.key);
        assert_eq!(setting.reference, None, "{:?}", setting.key);
        // Drift guard for the diagnostics-field → dotted-key table: the
        // origin is looked up under the table's dotted key (the three
        // runner rows display distinct labels but share the grouped
        // `jobs.runner` origin), so a mistyped key cannot hide behind the
        // Default fallback.
        assert!(
            runtime.resolved.provenance.get(lookup_key).is_some(),
            "no origin recorded for {lookup_key:?} (row {:?})",
            setting.key
        );
    }
}

#[test]
fn every_key_is_a_user_facing_key_in_table_order() {
    let runtime = runtime_from("", "", CliOverrides::default(), BTreeMap::new(), None);
    let explanation = explain(&runtime);
    for (setting, (field, lookup_key)) in explanation
        .settings
        .iter()
        .zip(saya_config::user_facing_keys())
    {
        // The three `[jobs.runner]` fields share one grouped lookup key, so
        // their rows display distinct labels; every other row displays the
        // lookup key itself.
        let expected = match *field {
            "jobs_runner_allow" => "jobs.runner.allow",
            "jobs_runner_program_dir" => "jobs.runner.program_dir",
            "jobs_runner_timeout_seconds" => "jobs.runner.timeout_seconds",
            _ => *lookup_key,
        };
        assert_eq!(setting.key, expected, "order follows the user-facing table");
    }
}
