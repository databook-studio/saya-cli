use super::*;

/// The parent's environment carries a real user config home with a
/// production profile in it; the nested child — its environment built,
/// not inherited — must resolve its user layer inside run state where
/// nothing exists, open the generated connections and find only the run
/// corpus, and be denied the planted real config by path. Asserted on
/// the child's own report of what it could open.
#[tokio::test]
async fn a_nested_saya_sees_no_production_profile_in_its_world() {
    // The real user config the child would resolve if its environment
    // were inherited, planted where the parent's SAYA_CONFIG_HOME points.
    let planted_home = leak("reentry-user-config");
    let user_saya = planted_home.join("saya");
    fs::create_dir_all(&user_saya).expect("the planted user config home");
    fs::write(
        user_saya.join("config.toml"),
        "[ai]\nmodel = \"prod-model\"\n",
    )
    .expect("the planted user config");
    fs::write(
        user_saya.join("connections.toml"),
        format!("[profiles.\"{PRODUCTION_PROFILE}\"]\npath = \"/data/warehouse.db\"\n"),
    )
    .expect("the planted production connections");
    // The parent's own env carries it — the child must not inherit it.
    unsafe { std::env::set_var("SAYA_CONFIG_HOME", &planted_home) };

    let battery = battery("reentry", "reentry-1");
    let state = nested_saya::prepare(
        &battery.run,
        &[endpoint("orchestrator", None)],
        &Default::default(),
        &[corpus("corp", "workspace/corpus/corp.db")],
    )
    .expect("the generated configs");
    let resolver = StaticCredentialSource::new(Vec::<(String, String)>::new());
    let cancellation = CancellationToken::new();
    // The question doubles as the probe path: the planted real user
    // connections, which the child must fail to open.
    let probe = user_saya.join("connections.toml");
    let outcome = nested_saya::ask(
        &battery.spawn,
        &scope(),
        &state,
        NestedAsk {
            profile: PROFILE,
            question: &probe.display().to_string(),
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
    let state_dir = state.state_dir();
    // The child's own user-layer resolution points inside run state —
    // the planted parent env did not reach it — and finds nothing there.
    assert_eq!(
        report_line(report, "env-pin:SAYA_CONFIG_HOME="),
        format!(
            "env-pin:SAYA_CONFIG_HOME={}",
            state_dir.join("config-home").display()
        ),
        "the child's config home is the run state's, not the parent's"
    );
    let user_config = report_line(report, "open:user-config:");
    let resolved = state_dir.join("config-home/saya/config.toml");
    assert!(
        user_config.contains(":denied(") && user_config.contains(&resolved.display().to_string()),
        "the child's user config resolves inside run state and finds nothing: {user_config}"
    );
    assert!(
        report_line(report, "open:user-connections:").contains(":denied("),
        "the child's user connections find nothing: {report}"
    );
    // The generated connections it actually opened hold only the corpus.
    assert!(
        report.contains(&format!(
            "open:connections:{}:ok",
            state.connections_toml().display()
        )),
        "the child opened the generated connections: {report}"
    );
    assert!(
        report.contains("[profiles.\"corp\"]"),
        "the child opened and read the run's corpus: {report}"
    );
    assert!(
        !report.contains(PRODUCTION_PROFILE),
        "no production profile anywhere in the child's world: {report}"
    );
    // The sandbox denies the planted real config by path outright — the
    // outside closure beside the inside one.
    let probe_line = report_line(report, "open:probe:");
    assert!(
        probe_line.contains(":denied("),
        "the real user config is unreachable by path: {probe_line}"
    );

    unsafe { std::env::remove_var("SAYA_CONFIG_HOME") };
    let _ = fs::remove_dir_all(battery.run.root());
    let _ = fs::remove_dir_all(&planted_home);
}
