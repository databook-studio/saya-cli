#[tokio::test]
async fn queue_slash_and_headless_agree_on_claim_ids_and_order() {
    let root = temp_root("queue_parity");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    // Seed three `Pending` knowledge items on distinct objects, directly
    // through the store (slash `remember` only confirms; the queue is the
    // candidate worklist). The queue reads `knowledge_items`, so a legacy
    // `propose_claim` row (which writes `contract_claims`, not `knowledge_items`)
    // would be invisible to it.
    let identity = identity_for(&runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    for table in ["a", "b", "c"] {
        let object = DatabaseObjectRef::new(
            profile.clone(),
            "analytics",
            "public",
            table,
            DatabaseObjectKind::Table,
        )
        .unwrap();
        let payload = ClaimPayload::table_alias(table).unwrap();
        let slot = KnowledgeSlot::TableAlias;
        let binding = SchemaBinding::derive(&slot, &payload).expect("slot/payload agree");
        store
            .put_knowledge_item(saya_store::KnowledgeItemRequest {
                object,
                slot,
                value: payload,
                source: ClaimOrigin::AssistantInferred,
                state: KnowledgeState::Pending,
                schema_binding_json: serde_json::to_string(&binding).unwrap(),
                fingerprint: unobserved_fingerprint(),
            })
            .await
            .unwrap();
    }

    let headless = run_headless(
        ContractsCommand::Queue {
            profile: None,
            limit: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (_cmd, code, out, err) = run_slash("/queue", &runtime, &store, RenderFormat::Text).await;

    assert_eq!(code, 0, "/queue stderr: {err}");
    // The translated slash command must equal the headless one — same operation.
    assert_eq!(
        _cmd,
        ContractsCommand::Queue {
            profile: None,
            limit: None,
        }
    );
    // Same claim ids, same order, byte for byte — a second queue read would
    // diverge here.
    assert_eq!(out, headless.1, "slash /queue diverged from headless queue");
    assert_eq!(err, headless.2);

    // The queue lists three `ki-…` candidates, in the same order on both paths.
    // (The queue orders oldest-first, then slot, then id — not by evidence,
    // which the legacy `contract_evidence` table carried; that table is gone.)
    let slash_ids: Vec<&str> = out
        .lines()
        .filter(|l| l.starts_with("ki-"))
        .map(|l| l.split_whitespace().next().unwrap_or(""))
        .collect();
    assert_eq!(slash_ids.len(), 3, "/queue output: {out}");
    let headless_ids: Vec<&str> = headless
        .1
        .lines()
        .filter(|l| l.starts_with("ki-"))
        .map(|l| l.split_whitespace().next().unwrap_or(""))
        .collect();
    assert_eq!(slash_ids, headless_ids, "order diverged");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 12. The TUI `/queue` adapter routes at the session's active profile, not the
//     configured default. Regression for the P2 defect where `with_profile`
//     (crates/saya-cli/src/interactive/tui/dispatch_actions.rs) fell through for
//     `Queue`, so a `/queue` parsed to `Queue { profile: None }` reached the
//     dispatcher un-stamped and resolved the configured default — reading another
//     profile's candidates. The queue is where a human confirms a claim, so
//     cross-profile isolation (an invariant with a named test in every layer
//     beneath) must hold at the adapter too.
//
//     The slash path parses `/queue` to `Queue { profile: None }` (test 11); the
//     TUI adapter's only addition is stamping the active profile onto that
//     `None`. This test proves the stamp is what routes the queue: with the
//     configured default set to `local` and a candidate seeded only under
//     `staging` (the profile a `/connect staging` would have made active),
//     `Queue { profile: Some("staging") }` — what the adapter now produces —
//     lists the candidate, while the un-stamped `Queue { profile: None }` the
//     broken adapter emitted resolves the default `local` and does not.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn tui_queue_routes_at_active_profile_not_configured_default() {
    let root = temp_root("queue_active_profile");
    // Two profiles; `local` is the configured default (via the `--profile` the
    // headless harness sets on load). `staging` is the profile a `/connect`
    // would have made active in the TUI — distinct from the default.
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
    // Migrate the pool and seed an (empty) cached schema for `staging` so its
    // candidates classify against a real cache state, not a missing one.
    let staging_identity = identity_for(&runtime, "staging");
    store
        .upsert_schema(&staging_identity, &SchemaTree::default())
        .await
        .unwrap();

    // Seed one `Pending` knowledge item under `staging` only, directly through the
    // store. The queue reads `knowledge_items`, so a legacy `propose_claim` row
    // (which writes `contract_claims`) would be invisible to it.
    let staging_profile = ProfileIdentity::parse(&staging_identity).unwrap();
    let object = DatabaseObjectRef::new(
        staging_profile.clone(),
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = ClaimPayload::table_alias("orders").unwrap();
    let slot = KnowledgeSlot::TableAlias;
    let binding = SchemaBinding::derive(&slot, &payload).expect("slot/payload agree");
    store
        .put_knowledge_item(saya_store::KnowledgeItemRequest {
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

    // The adapter's output: the active profile (`staging`) stamped onto the
    // `Queue { profile: None }` the slash parser produced. The candidate appears.
    let (active_code, active_out, active_err) = run_headless(
        ContractsCommand::Queue {
            profile: Some("staging".into()),
            limit: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(active_code, 0, "active queue stderr: {active_err}");
    assert!(
        active_out.contains("ki-"),
        "active (staging) queue should list the seeded candidate: {active_out}"
    );

    // The broken adapter's output: `Queue { profile: None }` un-stamped, which
    // resolves the configured default (`local`). The candidate must NOT appear —
    // it lives under `staging`. This is the divergence the stamp prevents.
    let (default_code, default_out, default_err) = run_headless(
        ContractsCommand::Queue {
            profile: None,
            limit: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(default_code, 0, "default queue stderr: {default_err}");
    assert!(
        !default_out.contains("ki-"),
        "un-stamped queue resolved the default (local) and must not show staging's \
         candidate — the bug the adapter's stamp prevents: {default_out}"
    );

    // Sanity: the default profile is `local`, and its queue is empty too.
    let (local_code, local_out, local_err) = run_headless(
        ContractsCommand::Queue {
            profile: Some("local".into()),
            limit: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(local_code, 0, "local queue stderr: {local_err}");
    assert!(
        !local_out.contains("ki-"),
        "local queue should be empty: {local_out}"
    );

    let _ = fs::remove_dir_all(root);
}
