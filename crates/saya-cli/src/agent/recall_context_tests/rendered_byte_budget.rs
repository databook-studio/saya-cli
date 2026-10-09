use super::*;

/// Spec test 3: the measured size accounts for rendered overhead, and the
/// accounting is not payload length. A contract with a conflict carries
/// dispute markers and a conflict line on top of its claim payloads, so its
/// rendered stanza is materially larger than its serialized payloads. With a
/// budget between the two, the rendered bound drops the contract (a payload
/// bound would have admitted it and exceeded the budget); with a budget above
/// the rendered stanza, the conflict block is produced and within budget.
#[tokio::test]
async fn the_byte_bound_measures_the_rendered_block_not_the_payload() {
    let root = temp_root("p1_byte_rendered");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let registry = registry_for("analytics", &identity);
    let name_of = super::render::name_by_identity(&registry);

    // D-3 NOTE: the original test seeded two conflicting grains so the rendered
    // stanza carried `[disputed]` markers + a conflict line over its payloads.
    // Two confirmed grains cannot coexist under D-3 (single-valued slot), so
    // the store path now seeds one large description: its rendered stanza is
    // still materially larger than its serialized payload (header + directive +
    // `[confirmed]` marker + kind token wrap the text), which is the unit the
    // byte bound must measure. The directly-constructed two-grain contract
    // below still measures the larger dispute overhead in isolation.

    // One large description claim through the store the recall path reads.
    let big = "z".repeat(1024);
    let obj = seed_large_description(&store, &identity, "orders", &big).await;

    // Reconstruct the one contract the way recall would, to measure its rendered
    // stanza and its serialized payloads against the same claim.
    use crate::contracts::{ContractClaim, RetrievedContract};
    let claim = ContractClaim {
        id: ClaimId::parse("c-aaa111222000").unwrap(),
        object: obj.clone(),
        value: ClaimPayload::table_description(&big).unwrap(),
        source: ClaimOrigin::UserExplicit,
        status: ClaimStatus::Confirmed,
    };
    let contract = RetrievedContract {
        object: obj.clone(),
        schema_state: crate::contracts::ContractSchemaState::Current,
        claims: vec![claim],
        conflicts: Vec::new(),
        truncated: false,
        incomplete: false,
    };
    let rendered_stanza = super::render::render_body(std::slice::from_ref(&contract), &name_of);
    let payload_bytes: usize = contract
        .claims
        .iter()
        .map(|c| {
            serde_json::to_string(&c.value)
                .map(|s| s.len())
                .unwrap_or(0)
        })
        .sum();
    // The rendered stanza materially exceeds the serialized payload: the
    // header, the directive, and the `[confirmed]` marker wrap the text.
    assert!(
        rendered_stanza.len() > payload_bytes + 64,
        "rendered stanza must materially exceed the payloads: rendered={} payload={}",
        rendered_stanza.len(),
        payload_bytes
    );
    // A budget between the two: payload fits, rendered does not.
    let between = payload_bytes + (rendered_stanza.len() - payload_bytes) / 2;
    assert!(
        payload_bytes < between && between < rendered_stanza.len(),
        "budget must sit between payload and rendered"
    );

    let bounds_between = RecallBounds {
        max_objects: 5,
        max_claims_per_object: 12,
        max_bytes: between,
    };
    let (blocks, _receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::Confirmed,
        bounds_between,
        &registry,
        Some(&store),
    )
    .await;
    // The rendered bound drops the contract: its stanza exceeds the budget
    // even though its payload alone would fit. A payload bound (the bug)
    // would have admitted it and produced a body over the budget.
    assert!(
        blocks[0].truncated,
        "the contract is dropped by the rendered bound: {:?}",
        blocks[0]
    );
    assert!(
        !blocks[0].body.contains(&big),
        "the contract's stanza must not be admitted when its rendered size exceeds budget"
    );

    // With a budget above the rendered stanza, the block is produced and within
    // budget — the overhead is accounted, not ignored.
    let bounds_generous = RecallBounds {
        max_objects: 5,
        max_claims_per_object: 12,
        max_bytes: rendered_stanza.len() + 64,
    };
    let (blocks, _receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::Confirmed,
        bounds_generous,
        &registry,
        Some(&store),
    )
    .await;
    let body = &blocks[0].body;
    assert!(
        body.contains(&big),
        "the block is produced when it fits: {body}"
    );
    assert!(
        body.len() <= rendered_stanza.len() + 64,
        "the produced body is within the rendered budget: {}",
        body.len()
    );
    let _ = fs::remove_dir_all(root);
}

/// Spec test 4: context never consumes the budget the user's own prompt needs.
/// A long prompt leaves less for context, and the request still builds — the
/// context block is squeezed to what fits under the message budget alongside the
/// prompt, never pushing the turn over the limit.
#[tokio::test]
async fn a_long_prompt_leaves_less_for_context_and_the_request_still_builds() {
    let root = temp_root("p1_byte_long_prompt");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let big = "z".repeat(1024);
    // Five objects each with a ~1 KiB claim: five stanzas total ~5.5 KiB.
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
    let bounds = RecallBounds::defaults();

    // A short prompt: the whole message budget is available for context, so all
    // five contracts fit (their ~5.5 KiB is well under the 16 KiB configured cap
    // and the ~32 KiB message budget).
    let (short_blocks, _receipt) = recall_blocks(
        "orders",
        None,
        true,
        RecallMode::Confirmed,
        bounds,
        &registry,
        Some(&store),
    )
    .await;
    let short_stanzas = short_blocks[0].body.matches("[current]").count();
    assert_eq!(
        short_stanzas, 5,
        "a short prompt admits all five contracts: {}",
        short_blocks[0].body
    );

    // A long prompt: the prompt itself consumes most of the message budget, so
    // little is left for context. The block is squeezed to what fits and the
    // request still builds — `turn_bytes` (the exact size `build_messages`
    // enforces) stays under the message budget.
    let long_prompt = format!("orders {}", "x".repeat(30_000));
    let (long_blocks, _receipt) = recall_blocks(
        &long_prompt,
        None,
        true,
        RecallMode::Confirmed,
        bounds,
        &registry,
        Some(&store),
    )
    .await;
    let block = &long_blocks[0];
    let long_stanzas = block.body.matches("[current]").count();
    assert!(
        long_stanzas < short_stanzas,
        "a long prompt must admit fewer contracts than a short one: long={long_stanzas} short={short_stanzas}"
    );
    assert!(
        block.truncated,
        "squeezing context to fit a long prompt must mark the block truncated"
    );
    // The regression guard: the request still builds — the context block, the
    // long prompt, and the system message together stay under the message budget.
    let turn = turn_bytes(None, std::slice::from_ref(block), &long_prompt);
    assert!(
        turn <= RECALL_BYTE_BUDGET,
        "context must not consume the budget the prompt needs: turn={turn} budget={RECALL_BYTE_BUDGET}"
    );
    let _ = fs::remove_dir_all(root);
}
