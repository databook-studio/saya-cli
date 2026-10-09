#[test]
fn candidate_reloads_the_real_sources_and_the_process_env_map_wins() {
    let user = temp_dir("user");
    std::fs::write(user.join("config.toml"), "[ai]\nmodel = \"from-file\"\n").unwrap();
    let cwd = temp_dir("cwd");
    let inputs = ReloadInputs {
        options: GlobalOptions::default(),
        cwd,
        user_dir: user,
        process_env: BTreeMap::from([("SAYA_MODEL".to_owned(), "from-env".to_owned())]),
    };
    let candidate = build_candidate(&inputs).unwrap();
    assert_eq!(candidate.resolved.ai.model, "from-env");
    assert!(candidate.resolved.ignored_project_overrides.is_empty());
}

#[test]
fn a_failed_build_leaves_the_callers_runtime_alone() {
    let user = temp_dir("invalid");
    std::fs::write(user.join("config.toml"), "[ai]\nmodel = \"m1\"\n").unwrap();
    let cwd = temp_dir("invalid-cwd");
    let inputs = ReloadInputs {
        options: GlobalOptions::default(),
        cwd,
        user_dir: user.clone(),
        process_env: BTreeMap::new(),
    };
    let held = build_candidate(&inputs).unwrap();
    std::fs::write(user.join("config.toml"), "[ai]\nmodel = \"unterminated\n").unwrap();
    let error = build_candidate(&inputs).unwrap_err();
    assert!(!error.is_empty());
    assert_eq!(
        held.resolved.ai.model, "m1",
        "the failed rebuild must not touch the runtime the caller already holds"
    );
}

#[test]
fn build_candidate_rereads_the_env_file_every_call() {
    let user = temp_dir("envfile-user");
    std::fs::write(user.join("config.toml"), "").unwrap();
    let env_file = temp_dir("envfile").join("overrides.env");
    std::fs::write(&env_file, "SAYA_AI_MODEL=first\n").unwrap();
    let inputs = ReloadInputs {
        options: GlobalOptions {
            env_file: Some(env_file),
            ..Default::default()
        },
        cwd: temp_dir("envfile-cwd"),
        user_dir: user,
        process_env: BTreeMap::new(),
    };
    let first = build_candidate(&inputs).unwrap();
    assert_eq!(first.resolved.ai.model, "first");
    std::fs::write(first_resolved_env_path(&inputs), "SAYA_AI_MODEL=second\n").unwrap();
    let second = build_candidate(&inputs).unwrap();
    assert_ne!(first.secret_values, second.secret_values);
    assert_eq!(
        second
            .secret_values
            .get("SAYA_AI_MODEL")
            .map(String::as_str),
        Some("second")
    );
}

fn first_resolved_env_path(inputs: &ReloadInputs) -> PathBuf {
    inputs.options.env_file.clone().unwrap()
}

// --- plan_reload: unchanged and section membership ---

#[test]
fn an_unchanged_runtime_yields_an_empty_plan_and_the_unchanged_line() {
    let old = runtime_from("[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 50\n", "");
    let state = state_with("m1");
    let plan = plan_reload(&old, &old, &state);
    assert!(plan.is_empty());
    assert_eq!(plan.render(), "Configuration is unchanged.");
}
