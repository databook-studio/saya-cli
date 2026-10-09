use super::*;

// P1 regression: a missing cache entry must not mute the model's memory.
//
// The bug (`resolve_profiles` collapsing `Ok(None)` to `SchemaTree::default()`)
// made a profile with claims but no cached schema classify every claim `Stale`,
// and stale contracts are excluded from the model — so a user who had never
// run `connection schema --refresh` saw an empty context block and the model
// quietly forgot everything, with schema drift blamed. The fix keeps `Missing`
// distinct: it classifies `LiveSchemaUnavailable`, which is *not* excluded, so
// the claim still reaches the model, plainly labelled.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------

/// D4e: the byte budget still holds at the caps with the directive present.
/// The largest legal block — `max_objects` objects each with
/// `max_claims_per_object` confirmed claims — still fits both the configured
/// `max_bytes` cap and the agent message budget, without raising any cap. The
/// directive is present once per object, so this also proves it costs the
/// budget once per contract, not once per claim.
#[tokio::test]
async fn byte_budget_holds_at_caps_with_directive_present() {
    let root = temp_root("p2a_budget_at_caps");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let bounds = RecallBounds::defaults();
    // max_objects objects, each with max_claims_per_object confirmed items. The
    // four columns c0..c3 are present so the column-description bindings
    // (Column{cJ, Exists}) validate against the live schema.
    let tables: Vec<Table> = (0..bounds.max_objects)
        .map(|i| {
            table_named_with(
                &format!("orders{i}"),
                &[
                    ("id", "bigint", false),
                    ("c0", "text", true),
                    ("c1", "text", true),
                    ("c2", "text", true),
                    ("c3", "text", true),
                ],
            )
        })
        .collect();
    let tree = SchemaTree {
        databases: vec![Database {
            name: "catalog".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables,
            }],
        }],
    };
    store.upsert_schema(identity.as_str(), &tree).await.unwrap();
    // D-3 NOTE: a multi-valued slot holds at most `MAX_MULTI_SLOT_VALUES` (4)
    // values, so 12 items per object must span distinct slots, not 12 aliases.
    // Seed 4 aliases + 4 descriptions + 4 column-descriptions (on 4 distinct
    // columns) per object — 12 items, all coexisting under D-3 cardinality.
    for i in 0..bounds.max_objects {
        let obj = object(&identity, &format!("orders{i}"));
        for j in 0..4 {
            put_item(
                &store,
                &obj,
                ClaimPayload::table_alias(format!("a{i}_{j}")).unwrap(),
                KnowledgeState::Active,
            )
            .await;
            put_item(
                &store,
                &obj,
                ClaimPayload::table_description(format!("d{i}_{j}")).unwrap(),
                KnowledgeState::Active,
            )
            .await;
            put_item(
                &store,
                &obj,
                ClaimPayload::column_description(format!("c{j}"), format!("col {j}")).unwrap(),
                KnowledgeState::Active,
            )
            .await;
        }
    }

    let registry = registry_for("analytics", &identity);
    let (blocks, _receipt) = recall_blocks(
        "orders",
        None,
        true,
        RecallMode::Confirmed,
        bounds,
        &registry,
        Some(&store),
    )
    .await;
    assert_eq!(blocks.len(), 1);
    let block = &blocks[0];
    // No bound was raised: the largest legal block still fits the configured
    // cap and is not truncated by the byte bound (only count bounds apply).
    assert!(
        !block.truncated,
        "the largest legal block fits without truncation: {}",
        block.body.len()
    );
    assert!(
        block.body.len() <= bounds.max_bytes,
        "body is within the byte cap: {} <= {}",
        block.body.len(),
        bounds.max_bytes
    );
    // The directive is present once per object — `max_objects` times — so the
    // cost is per contract, not per claim.
    let directive_count = block
        .body
        .matches(super::render::CONFIRMED_DIRECTIVE)
        .count();
    assert_eq!(
        directive_count,
        bounds.max_objects,
        "directive appears once per object, not per claim: {directive_count} in {} bytes",
        block.body.len()
    );
    // The whole turn (system + block + prompt) still fits the message budget,
    // measured with the same accounting build_messages enforces.
    let turn = turn_bytes(None, std::slice::from_ref(block), "orders");
    assert!(
        turn <= RECALL_BYTE_BUDGET,
        "turn fits the conversation budget: {turn} <= {RECALL_BYTE_BUDGET}"
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn missing_cache_entry_keeps_the_claim_labelled_not_muted_as_stale() {
    let root = temp_root("p1_missing_cache");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    // Open the store but DO NOT cache a schema — the `Missing` case. The
    // `store_at` helper caches `SchemaTree::default()`, which would hide the
    // bug (an empty *cached* tree is `Available`, not `Missing`).
    let store = SqliteStateStore::new(&db);
    let obj = object(&identity, "orders");
    let fp = live_fingerprint(&orders_table());
    remember_confirmed_default_time_column(&store, &obj, &fp, "created_at").await;

    let registry = registry_for("analytics", &identity);
    let (blocks, _receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    // The bug produced an empty block (everything classified Stale → excluded).
    // The fix keeps the contract: one block, labelled `live_schema_unavailable`,
    // so the model still sees the claim and knows the schema could not be vouched
    // for — never a silent empty memory.
    assert_eq!(
        blocks.len(),
        1,
        "a missing cache keeps the claim; the bug would have muted it"
    );
    let body = &blocks[0].body;
    assert!(
        body.contains("created_at"),
        "the claim still reaches the model: {body}"
    );
    assert!(
        body.contains("live_schema_unavailable"),
        "the missing cache is labelled, not read as current: {body}"
    );
    // The diagnostic must not blame drift: the column is not gone, the schema
    // was simply never cached.
    assert!(
        !body.contains("possibly out of date"),
        "a missing cache is not staleness: {body}"
    );
    let _ = fs::remove_dir_all(root);
}
