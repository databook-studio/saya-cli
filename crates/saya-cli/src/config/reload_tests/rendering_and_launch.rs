#[test]
fn ignored_project_overrides_are_carried_and_rendered_once() {
    let old = runtime_from_layers("[run]\nmax_rows = 50\n", None, "");
    let new = runtime_from_layers(
        "[run]\nmax_rows = 50\n",
        Some("[ai]\nbase_url = \"http://project:8080/v1\"\n"),
        "",
    );
    let plan = plan_reload(&old, &new, &state_with("m"));
    assert_eq!(
        plan.ignored_project_overrides,
        vec!["ai.base_url".to_owned()]
    );
    assert!(
        plan.is_empty(),
        "a reverted project override changed no value, so only the ignored line renders"
    );
    let render = plan.render();
    assert!(render.starts_with("Configuration is unchanged."));
    assert_eq!(
        render
            .matches("Ignored from the project file: ai.base_url")
            .count(),
        1
    );
    assert!(render.contains("(security-critical; see saya config show --explain)."));
}

#[test]
fn a_secret_never_reaches_the_plan_or_the_render() {
    let user = temp_dir("sentinel");
    std::fs::write(user.join("config.toml"), "").unwrap();
    let inputs = ReloadInputs {
        options: GlobalOptions::default(),
        cwd: temp_dir("sentinel-cwd"),
        user_dir: user,
        process_env: BTreeMap::from([(
            "SAYA_AI_API_KEY".to_owned(),
            "sk-live-sentinel-123".to_owned(),
        )]),
    };
    let candidate = build_candidate(&inputs).unwrap();
    let old = runtime_from("", "");
    let plan = plan_reload(&old, &candidate, &state_with("m"));
    for line in plan
        .applied
        .iter()
        .chain(&plan.reseeded)
        .chain(&plan.pending)
    {
        let json = format!("{} {}", line.before, line.after);
        assert!(!json.contains("sk-live-sentinel-123"), "{json}");
    }
    let render = plan.render();
    assert!(!render.contains("sk-live-sentinel-123"));
    assert!(render.contains("env:SAYA_AI_API_KEY"));
}

#[test]
fn a_mixed_plan_renders_the_three_sections_in_key_order() {
    let connections = "[profiles.a]\ntype = \"sqlite\"\npath = \"/tmp/a.sqlite\"\n[profiles.b]\ntype = \"sqlite\"\npath = \"/tmp/b.sqlite\"\n";
    let old = runtime_from(
        "default_profile = \"a\"\n[ai]\nallow_data_sharing = false\n[run]\nmax_iterations = 12\nmax_rows = 50\n",
        connections,
    );
    let new = runtime_from(
        "default_profile = \"b\"\n[ai]\nallow_data_sharing = true\n[run]\nmax_iterations = 20\nmax_rows = 100\n[host_commands]\ntimeout_seconds = 99\n",
        connections,
    );
    let mut session = state_with("m1");
    session.profile = Some("a".to_owned());
    let plan = plan_reload(&old, &new, &session);
    assert_eq!(
        plan.render(),
        "Reloaded configuration: 2 applied, 1 re-seeded, 2 pending.\n\
         Applied from the next turn:\n\
         \x20 max_iterations: 12 -> 20\n\
         \x20 max_rows: 50 -> 100\n\
         Updated for this session:\n\
         \x20 profile_name: a -> b\n\
         Not applied in this session:\n\
         \x20 allow_data_sharing: false -> true — data sharing is fixed for this session; use /privacy to change it here, or start a new session\n\
         \x20 host_commands: {\"pass_env\":[],\"timeout_seconds\":600} -> {\"pass_env\":[],\"timeout_seconds\":99} — takes effect in a new session"
    );
}

// --- the class table ---

#[test]
fn the_class_table_covers_every_diagnostics_field() {
    let value = serde_json::to_value(resolved_from("", None, "").redacted_diagnostics()).unwrap();
    let keys: BTreeSet<String> = value.as_object().unwrap().keys().cloned().collect();
    let table: BTreeSet<&str> = CLASS_TABLE.iter().map(|(name, _)| *name).collect();
    for key in DEFAULTS_TO_PENDING {
        assert!(
            !table.contains(key),
            "{key} must stay out of the table so it defaults to pending"
        );
        assert!(
            keys.contains(*key),
            "{key} is no longer a diagnostics field"
        );
    }
    let unclassified: Vec<&String> = keys
        .iter()
        .filter(|key| !table.contains(key.as_str()) && !DEFAULTS_TO_PENDING.contains(&key.as_str()))
        .collect();
    assert!(
        unclassified.is_empty(),
        "new diagnostics fields need a conscious classification: {unclassified:?}"
    );
    for name in &table {
        assert!(
            keys.contains(*name),
            "class table key {name} is not a diagnostics field"
        );
    }
}

#[test]
fn an_unknown_diagnostics_key_defaults_to_pending() {
    assert!(matches!(
        classify_key("some_future_field"),
        Class::Unrecognised
    ));
    let old = runtime_from("", "");
    let new = runtime_from("", "");
    let mut plan = ReloadPlan::default();
    let change = ChangeLine::new("some_future_field", &Value::from(1), &Value::from(2), None);
    classify_change(&change, &old, &new, &state_with("m"), &mut plan);
    assert_eq!(plan.pending.len(), 1);
    assert_eq!(plan.pending[0].key, "some_future_field");
}

#[test]
fn from_launch_reads_the_current_environment() {
    let inputs = ReloadInputs::from_launch(&GlobalOptions::default());
    assert_eq!(inputs.cwd, PathBuf::from("."));
    assert_eq!(
        inputs
            .user_dir
            .file_name()
            .map(std::ffi::OsStr::to_string_lossy),
        Some(std::borrow::Cow::Borrowed("saya"))
    );
    assert!(
        !inputs.process_env.is_empty(),
        "production reads the real env"
    );
}
