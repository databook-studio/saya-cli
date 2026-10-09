use super::*;

// P1 byte-budget: the bound is on the rendered block, against what is left of
// the agent message budget, and it bounds the first claim too.
//
// The old bound in `assemble` measured serialized claim payloads and skipped the
// first claim of every object, so it was neither the right unit nor a real bound.
// The fix renders the body, measures the rendered block (headers, markers,
// conflict lines, the wrapper) with the same accounting `build_messages`
// enforces, and drops contracts from the end until it fits — including the
// first. The budget is the agent message budget less the system prompt and the
// user's own question, so context never crowds out the question.
// ---------------------------------------------------------------------------

/// Spec test 1: a single claim larger than the budget is **omitted**, and the
/// result is marked truncated. The old code admitted the first claim of every
/// object regardless of size; this is exactly that case, and the fix must not
/// let it through.
#[tokio::test]
async fn a_single_oversized_claim_is_omitted_and_the_block_is_marked_truncated() {
    let root = temp_root("p1_byte_single");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let big = "z".repeat(1024);
    seed_large_description(&store, &identity, "orders", &big).await;

    let registry = registry_for("analytics", &identity);
    // A budget far smaller than the claim's rendered line (~1 KiB) but large
    // enough that the object header alone would fit if the claim were small —
    // so the omission is the claim's size, not the header's.
    let bounds = RecallBounds {
        max_objects: 5,
        max_claims_per_object: 12,
        max_bytes: 512,
    };
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
    // The contract's rendered stanza (header + the 1 KiB claim line) exceeds
    // 512, so no contract fits. The block is still produced, marked truncated,
    // and the oversized claim never reaches the body.
    assert_eq!(
        blocks.len(),
        1,
        "a truncated block is produced, not silence"
    );
    assert!(
        blocks[0].truncated,
        "an oversized first claim must mark the block truncated"
    );
    assert!(
        !blocks[0].body.contains(&big),
        "the oversized claim must be omitted from the body: {}",
        blocks[0].body
    );
    let _ = fs::remove_dir_all(root);
}

/// Spec test 2: five objects each with one oversized claim produce a block within
/// budget, not five unbounded claims. The old code admitted the first claim of
/// every object regardless of size — five objects meant five unbounded claims.
/// The fix drops contracts from the end until the rendered block fits, so the
/// block carries only what fits and is marked truncated.
#[tokio::test]
async fn five_objects_with_oversized_claims_do_not_admit_five_unbounded_claims() {
    let root = temp_root("p1_byte_five");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let big = "z".repeat(1024);
    // Five objects, each with one claim whose rendered line is ~1 KiB. The
    // objects share one schema tree so each reads `current` (this test is about
    // bytes, not staleness).
    let tables: Vec<Table> = (0..5)
        .map(|i| table_named_with(&format!("orders{i}"), &[("id", "bigint", false)]))
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
    for i in 0..5 {
        let obj = object(&identity, &format!("orders{i}"));
        let _ = live_fingerprint(&table_named_with(
            &format!("orders{i}"),
            &[("id", "bigint", false)],
        ));
        put_item(
            &store,
            &obj,
            ClaimPayload::table_description(&big).unwrap(),
            KnowledgeState::Active,
        )
        .await;
    }

    let registry = registry_for("analytics", &identity);
    // A budget that admits a couple of the 1 KiB stanzas but not all five. The
    // old code would have admitted all five first claims (~5 KiB), far over 2300.
    let bounds = RecallBounds {
        max_objects: 5,
        max_claims_per_object: 12,
        max_bytes: 2300,
    };
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
    let block = &blocks[0];
    // The block is within the configured byte budget...
    assert!(
        block.body.len() <= 2300,
        "the rendered body must be within the byte budget: {}",
        block.body.len()
    );
    //...and it does not carry five claims — the bound dropped the excess. Each
    // object's header line is `catalog.public.ordersN  [current]...`; count
    // them to assert how many stanzas survived.
    let stanza_count = block.body.matches("[current]").count();
    assert!(
        stanza_count < 5,
        "five oversized claims must not all be admitted: {stanza_count} stanzas in {}",
        block.body
    );
    assert!(
        block.truncated,
        "dropping contracts to fit the budget must mark the block truncated"
    );
    let _ = fs::remove_dir_all(root);
}
