/// The deterministic remap regression (A922-1, D1): the target resolved at
/// admission is the one the command carries — so when the ordinary CLI
/// remaps the investigation's binding to a non-allowlisted profile between
/// resolution and execution (the queued-call window), the run refuses as
/// stale and the other database's row never appears; and a resolution taken
/// after the remap refuses outright. The rebind is the same operation the
/// CLI drives (`--connection other --revalidate`), run in-process.
#[tokio::test]
async fn a_binding_remapped_under_a_queued_replay_never_runs() {
    const SENTINEL: &str = "OTHER-SYNTHETIC-SENTINEL-4";
    let root = std::env::temp_dir().join(format!("saya-mcp-remap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let local_db = root.join("data.sqlite3");
    let other_db = root.join("other.sqlite3");
    let mut runtime = fixture_runtime(
        &format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n\n\
             [profiles.other]\ntype = 'sqlite'\npath = '{}'\n",
            local_db.display(),
            other_db.display(),
        ),
        "default_profile = 'local'\n",
    );
    runtime.investigations_root = root.join("investigations");
    std::fs::create_dir_all(&runtime.investigations_root).unwrap();
    // Both databases carry an `events` table with the same schema; the
    // non-allowlisted one holds a row the allowlisted one does not, so a
    // wrongly-targeted replay is visible in the payload.
    seed_events(&local_db, "benign").await;
    seed_events(&other_db, SENTINEL).await;
    let repo = InvestigationRepository::new(runtime.investigations_root.clone());
    let state_db = saya_store::SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&runtime, &state_db).await;
    let parsed = InvestigationId::parse(&id).unwrap();

    let context = super::context::McpContext {
        runtime: runtime.clone(),
        store: saya_store::SqliteStateStore::new(root.join("state.sqlite3")),
        replay_slot: tokio::sync::Mutex::new(()),
    };
    let policy = ServePolicy::resolve(&runtime, &["local".to_owned()], None).unwrap();
    use super::replay_tools::resolve_target;

    // Admission: the binding names `local`, the only allowlisted profile —
    // the target resolves to the concrete name the command will carry.
    let target = resolve_target(&policy, &context, &id, None).expect("the binding is allowlisted");
    assert_eq!(target, "local");

    // The ordinary CLI remap: the same run operation the CLI drives,
    // `--connection other --revalidate`, succeeds and rewrites the binding
    // to the profile this server does not serve.
    capture_output_start();
    let rebind = crate::commands::run_investigation_outcome(
        crate::cli::InvestigationCommand::Run {
            id: id.clone(),
            connection: Some("other".to_owned()),
            revalidate: true,
            report: None,
            rows: None,
            overwrite: false,
            params: Vec::new(),
        },
        &runtime,
        RenderFormat::Text,
        false,
        &state_db,
    )
    .await
    .unwrap();
    let (out, err) = capture_output_take();
    assert_eq!(rebind.code, 0, "the CLI rebind succeeds: {out}{err}");
    let moved = repo.get_binding(&parsed).unwrap().expect("binding exists");
    assert_eq!(
        moved.profile, "other",
        "the rebind moved the binding off the allowlist"
    );

    // The queued call's resolution now happens (as the flow does, after the
    // wait): the binding names a profile outside the allowlist — refused
    // before anything runs, the name never echoed.
    let refused = resolve_target(&policy, &context, &id, None)
        .expect_err("the remapped binding refuses at resolution");
    assert!(
        refused.contains("profile not available"),
        "the refusal names the gate: {refused}"
    );
    assert!(
        !refused.contains("other") && !refused.contains(SENTINEL),
        "neither the profile nor the other database's data is echoed: {refused}"
    );

    // And when the resolution raced ahead of the remap, the command the
    // adapter builds carries the resolved target explicitly, so the re-read
    // binding is stale against it — refused, never executed.
    capture_output_start();
    let outcome = crate::commands::run_investigation_outcome(
        crate::cli::InvestigationCommand::Run {
            id: id.clone(),
            connection: Some(target),
            revalidate: false,
            report: None,
            rows: None,
            overwrite: false,
            params: Vec::new(),
        },
        &runtime,
        RenderFormat::Text,
        false,
        &state_db,
    )
    .await
    .unwrap();
    let (out, err) = capture_output_take();
    assert_ne!(
        outcome.code, 0,
        "the remapped binding never runs against the resolved target: {out}{err}"
    );
    assert!(outcome.replay.is_none(), "a refusal carries no replay");
    assert!(
        out.contains("target changed") || err.contains("target changed"),
        "the refusal is the target staleness: {out}{err}"
    );
    assert!(
        !out.contains(SENTINEL) && !err.contains(SENTINEL),
        "the other database's row never leaves its database: {out}{err}"
    );
    let _ = std::fs::remove_dir_all(root);
}
