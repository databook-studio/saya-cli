/// `decide --profile <name>` reaches a claim in a non-active profile (spec
/// invariant 1b). `review` was not profile-scoped; if `decide` could not reach
/// a claim outside the active profile by name, that would be a real capability
/// loss and the slice must stop. It can, so retiring `review` is safe.
#[tokio::test]
async fn decide_profile_flag_reaches_a_claim_outside_the_active_profile() {
    let root = temp_root("decide_cross_profile");
    // Two profiles; `local` is the configured default (selected via `--profile`),
    // `staging` is a second profile whose claim is NOT reachable from the
    // default. `runtime_at`/`runtime_for_scope` build a fresh runtime with no
    // profile selection, which a two-profile file rejects (`MissingProfile`), so
    // this test builds the runtime inline — mirroring the cross-profile harness
    // in `contracts_slash_parity.rs` — and opens its own store.
    let local_db = root.join("local.sqlite3");
    let staging_db = root.join("staging.sqlite3");
    fs::write(&local_db, b"").unwrap();
    fs::write(&staging_db, b"").unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n\n\
             [profiles.staging]\ntype = 'sqlite'\npath = '{}'\n",
            local_db.display(),
            staging_db.display(),
        ),
    )
    .unwrap();
    let options = saya_cli::GlobalOptions {
        connections: Some(connections),
        profile: Some("local".into()),
        ..Default::default()
    };
    let runtime = load_with_sources(&options, &root, &root, BTreeMap::new()).unwrap();
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    // Migrate the pool and seed an empty cached schema for `staging` so its
    // candidate classifies against a real cache state and the confirm
    // revalidation has a live table to revalidate against.
    let staging_identity = identity_for(&runtime, "staging");
    store
        .upsert_schema(&staging_identity, &orders_schema())
        .await
        .unwrap();

    // Seed a candidate under `staging` only — directly, keyed by staging's
    // identity, so it is invisible to the default (`local`) profile.
    let staging_profile = ProfileIdentity::parse(&staging_identity).unwrap();
    let object = DatabaseObjectRef::new(
        staging_profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = alias_payload();
    let slot = KnowledgeSlot::TableAlias;
    let binding = SchemaBinding::derive(&slot, &payload).expect("slot/payload agree");
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object: object.clone(),
            slot,
            value: payload,
            source: ClaimOrigin::AssistantInferred,
            state: KnowledgeState::Pending,
            schema_binding_json: serde_json::to_string(&binding).unwrap(),
            fingerprint: unobserved_fingerprint(),
        })
        .await
        .unwrap();
    let staging_id = store
        .knowledge_for_object(&object)
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.slot == KnowledgeSlot::TableAlias)
        .expect("staging candidate seeded")
        .id;
    let staging_id = ClaimId::parse(&staging_id).expect("ki id parses");
    let prefix = staging_id.as_str().chars().take(6).collect::<String>();

    // Without `--profile`, `decide` resolves the default (`local`) and must NOT
    // find the staging claim — proving the claim is genuinely outside the
    // active profile (the thing `review`'s lack of scoping cannot address).
    let (default_code, default_out, default_err) = run(
        ContractsCommand::Decide {
            prefix: prefix.clone(),
            decision: ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        default_code, 0,
        "decide without --profile must not reach a non-active profile's claim: {default_out}{default_err}"
    );
    let default_combined = format!("{default_out}{default_err}");
    assert!(
        default_combined.contains("no claim matches"),
        "default-profile refusal must say no match: {default_combined}"
    );

    // With `--profile staging`, the staging claim IS reachable and confirmable.
    let (code, out, err) = run(
        ContractsCommand::Decide {
            prefix,
            decision: ReviewDecisionArg::Confirm,
            profile: Some("staging".into()),
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(
        code, 0,
        "decide --profile staging must reach the staging claim: {err}"
    );
    assert!(
        out.contains("confirmed"),
        "staging candidate -> confirmed: {out}"
    );

    // The staging claim alone changed; it is now Active.
    let items = store.knowledge_for_object(&object).await.unwrap();
    assert_eq!(items.len(), 1, "no new row: {items:?}");
    assert_eq!(
        items[0].state,
        KnowledgeState::Active,
        "the staging claim was confirmed: {items:?}"
    );

    let _ = fs::remove_dir_all(root);
}
