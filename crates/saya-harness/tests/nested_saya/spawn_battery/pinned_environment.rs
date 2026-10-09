use super::*;

#[tokio::test]
async fn the_pinned_environment_resolves_inside_the_run_state() {
    let battery = battery("pins", "pins-1");
    let state = nested_saya::prepare(
        &battery.run,
        &[endpoint("orchestrator", None)],
        &Default::default(),
        &[corpus("corp", "workspace/corpus/corp.db")],
    )
    .expect("the generated configs");
    let resolver = StaticCredentialSource::new(Vec::<(String, String)>::new());
    let cancellation = CancellationToken::new();
    let outcome = nested_saya::ask(
        &battery.spawn,
        &scope(),
        &state,
        NestedAsk {
            profile: PROFILE,
            question: "count orders per region",
            credentials: &[],
            resolver: &resolver,
        },
        DEFAULT_TIMEOUT,
        &cancellation,
    )
    .await
    .expect("the nested ask must run");
    assert_eq!(outcome.exit_code, Some(0), "the child must have run");

    let report = &outcome.stdout.text;
    let state_dir = battery.run.state();
    for var in ["SAYA_CONFIG_HOME", "SAYA_STATE_DB", "SAYA_SESSION_DIR"] {
        let line = report_line(report, &format!("env-pin:{var}="));
        let value = PathBuf::from(
            line.strip_prefix(&format!("env-pin:{var}="))
                .expect("the pin's value"),
        );
        assert!(
            value.starts_with(state_dir),
            "{var}={value:?} must resolve inside runs/<id>/state ({})",
            state_dir.display()
        );
    }
    // The child's own user-config resolution — the path its
    // SAYA_CONFIG_HOME implies — is inside run state too, and finds
    // nothing there: absence, not denial (DESIGN §6.4).
    let user_config = report_line(report, "open:user-config:");
    let resolved = state_dir.join("config-home/saya/config.toml");
    assert!(
        user_config.contains(":denied(") && user_config.contains(&resolved.display().to_string()),
        "the child's user layer resolves inside run state: {user_config}"
    );

    let _ = fs::remove_dir_all(battery.run.root());
}
