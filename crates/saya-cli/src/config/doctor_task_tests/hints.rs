#[test]
fn the_provider_fail_hint_names_the_user_config_file_and_the_model_flag() {
    let runtime = runtime_from("[ai]\nmodel = \"\"\n", "", BTreeMap::new());
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    let provider = check_named(&readiness, "provider");
    assert_eq!(provider.status, CheckStatus::Fail);
    assert_eq!(
        provider.hint.as_deref(),
        Some(format!("Set [ai] model in {}, or pass --model.", user_config_file()).as_str())
    );
}

#[test]
fn the_missing_credentials_hint_names_the_user_file_and_the_export_alternative() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    let credentials = check_named(&readiness, "credentials");
    assert_eq!(credentials.status, CheckStatus::Fail);
    assert_eq!(
        credentials.hint.as_deref(),
        Some(
            format!(
                "Set [ai] api_key = {{ env = \"NAME\" }} in {} (or export SAYA_AI_API_KEY).",
                user_config_file()
            )
            .as_str()
        )
    );
}

#[test]
fn the_unresolved_credentials_hint_names_the_variable_and_the_label_only() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\napi_key = { env = \"SAYA_TEST_AI_KEY\" }\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    let credentials = check_named(&readiness, "credentials");
    assert_eq!(credentials.status, CheckStatus::Fail);
    // The exact text: the variable name and the redacted label — a secret
    // value has nowhere to appear because the hint is fully pinned.
    assert_eq!(
        credentials.hint.as_deref(),
        Some(
            "Export SAYA_TEST_AI_KEY in your shell or --env-file; the reference is env:SAYA_TEST_AI_KEY."
        )
    );
}

#[test]
fn the_endpoint_warning_hint_points_at_the_user_config_file() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai_compatible\"\nmodel = \"m\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    let endpoint = check_named(&readiness, "endpoint");
    assert_eq!(endpoint.status, CheckStatus::Warn);
    assert_eq!(
        endpoint.hint.as_deref(),
        Some(format!("Set [ai] base_url in {}.", user_config_file()).as_str())
    );
}

#[test]
fn ready_checks_carry_no_hint_and_the_wire_omits_the_key() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"qwen2.5-coder:14b\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    assert!(readiness.ready);
    for check in &readiness.checks {
        assert_eq!(check.hint, None, "{:?}", check.name);
    }
    let serialized = serde_json::to_string(&readiness).unwrap();
    assert!(!serialized.contains("\"hint\""), "{serialized}");
}

#[test]
fn the_profile_fail_hint_names_the_setup_command() {
    let runtime = runtime_from("", "", BTreeMap::new());
    let readiness = readiness(&runtime, DoctorTask::Query, Path::new("/"));
    let profile = check_named(&readiness, "profile");
    assert_eq!(profile.status, CheckStatus::Fail);
    assert_eq!(
        profile.hint.as_deref(),
        Some(
            "Add a profile to connections.toml or pass --connections/--profile; \
             `saya setup` can do it."
        )
    );
}

#[test]
fn the_secrets_fail_hint_says_to_export_without_printing_values() {
    let runtime = runtime_from(
        "default_profile = \"analytics\"\n",
        "[profiles.analytics]\ntype = \"postgresql\"\nhost = \"localhost\"\ndatabase = \"warehouse\"\n\
         user = \"u\"\npassword = { env = \"SAYA_TEST_UNRESOLVED\" }\n",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Query, Path::new("/"));
    let secrets = check_named(&readiness, "secrets");
    assert_eq!(secrets.status, CheckStatus::Fail);
    assert_eq!(
        secrets.hint.as_deref(),
        Some("Export the variables named above; values are never printed.")
    );
}

#[test]
fn the_workspace_fail_hint_says_how_to_launch() {
    let dir = temp_dir("worktree-hint");
    if inside_a_worktree(&dir) {
        eprintln!(
            "skipping: {} unexpectedly sits inside a git worktree on this machine",
            dir.display()
        );
        return;
    }
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"m\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Coding, &dir);
    let workspace = check_named(&readiness, "workspace");
    assert_eq!(workspace.status, CheckStatus::Fail);
    assert_eq!(
        workspace.hint.as_deref(),
        Some(
            "Launch inside a git worktree or pass --workspace <dir> \
             (not available to config doctor: run saya from the project)."
        )
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_text_shape_prints_a_fix_line_under_a_check_with_a_hint() {
    let checks = vec![TaskCheck {
        name: "credentials".into(),
        status: CheckStatus::Fail,
        detail: "no api_key reference configured — requests to openai will be unauthenticated"
            .into(),
        hint: Some("Set [ai] api_key = { env = \"NAME\" } in /u/.config/saya/config.toml (or export SAYA_AI_API_KEY).".into()),
    }];
    let readiness = TaskReadiness {
        task: "ai",
        ready: false,
        checks,
    };
    let rendered = render_event(&event_from(&readiness), RenderFormat::Text);
    assert_eq!(
        rendered.stdout,
        "task ai: NOT ready\n\
         ✗ credentials: no api_key reference configured — requests to openai will be unauthenticated\n\
         \x20  fix: Set [ai] api_key = { env = \"NAME\" } in /u/.config/saya/config.toml (or export SAYA_AI_API_KEY).\n"
    );
}
