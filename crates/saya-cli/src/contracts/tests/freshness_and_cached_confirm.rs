/// Like [`recall_request`] but lets a test name the policy and the "now" the
/// freshness bound compares against. The default helper fixes both for the
/// common (model-path, fresh) case; the freshness tests need to vary them.
fn recall_request_with_freshness<'a>(
    profiles: &'a [ProfileIdentity],
    schemas: &'a [(ProfileIdentity, SchemaAvailability)],
    terms: &'a [String],
    allow_database_context: bool,
    bounds: RecallBounds,
    now_unix_ms: i64,
    policy: RetrievalPolicy,
) -> RecallRequest<'a> {
    RecallRequest {
        profiles,
        explicit_refs: &[],
        terms,
        allow_database_context,
        schemas,
        now_unix_ms,
        bounds,
        recall_mode: RecallMode::Confirmed,
        admit_candidate: None,
        policy,
    }
}

// ---------------------------------------------------------------------------
// P1 wiring: confirming a stale claim revalidates it against the cached
// schema (the claim's own profile), so it reads Current on the next read
// instead of bouncing back to Stale. See `contract_revalidate.rs` for the
// store-layer behaviour these exercise through the `confirm` wrapper.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn confirm_revalidates_a_stale_claim_against_the_cached_schema() {
    let root = temp_root("confirm_revalidates");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    // Under D-4 a "stale" fact is `Active` (state) reading `Invalid` (computed)
    // against a drifted schema. Seed an Active `default_time_column` on
    // `created_at`, then cache a schema that has it — so the binding reads
    // `Valid` and confirm has a live table to revalidate against.
    let id = seed_active_time_item(&store, &obj, "created_at").await;
    let base = schema_tree_for(&[(
        "orders",
        table(&[("id", "bigint", false), ("created_at", "timestamp", false)]),
    )]);
    store.upsert_schema(p.as_str(), &base).await.unwrap();

    // Confirming revalidates: it refreshes the binding/fingerprint version and
    // keeps the item `Active` against the live schema, so the next read is
    // `Current` (Valid). The item was already Active — the point is that
    // confirm re-checks against the cached schema rather than trusting a stored
    // verdict, and a valid binding reads Valid after.
    let confirmed = confirm(&store, &id).await.unwrap();
    assert_eq!(confirmed.status, ClaimStatus::Confirmed);
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Active);

    // The next read classifies it Current: the binding validates against the
    // cached schema.
    let shown = show(
        &store,
        &obj,
        &avail(base),
        RetrievalPolicy::ForHumanReview,
        FRESH_NOW,
    )
    .await
    .unwrap()
    .expect("a confirmed-against-schema contract is shown");
    assert_eq!(
        shown.schema_state,
        ContractSchemaState::Current,
        "a revalidated item reads Current, not Stale"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn confirm_refuses_a_stale_claim_with_no_cached_schema() {
    let root = temp_root("confirm_no_schema");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    // An Active fact (the "stale"-analog) with no cached schema: confirm is a
    // re-verification, and there is nothing to verify against — refuse, do not
    // rubber-stamp a belief we cannot check.
    let id = seed_active_time_item(&store, &obj, "created_at").await;
    store.invalidate_schema(p.as_str()).await.unwrap();

    let err = confirm(&store, &id).await.unwrap_err();
    assert_eq!(
        err,
        ContractOpError::SchemaUnavailable,
        "an existing fact cannot be reconfirmed without a schema"
    );
    // The item is unchanged — still Active, nothing written.
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Active);

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn confirm_refuses_a_stale_claim_whose_referenced_column_is_gone() {
    let root = temp_root("confirm_column_gone");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let id = seed_active_time_item(&store, &obj, "created_at").await;

    // Cache the drifted schema — `orders` still exists, but `created_at` is
    // gone. The item's `Column { created_at, Time }` binding reads `Invalid`,
    // so confirm must refuse rather than revive a fact whose dependency died.
    let drifted = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    store.upsert_schema(p.as_str(), &drifted).await.unwrap();

    let err = confirm(&store, &id).await.unwrap_err();
    assert_eq!(
        err,
        ContractOpError::ColumnGone,
        "do not revive a fact whose referenced column is gone"
    );
    // The item is unchanged — still Active, nothing written.
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Active);

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn confirm_a_candidate_is_status_only_and_needs_no_schema() {
    let root = temp_root("confirm_candidate");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");

    // A `Pending` candidate with no cached schema. Confirming it is a user
    // assertion — the user is the source, so it does not need a schema, and
    // must not require one. The item will read `SchemaUnavailable` post-confirm
    // (honest), never a false `Current`.
    store.invalidate_schema(p.as_str()).await.unwrap();
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("the orders table").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let id = item_id_for(&store, &obj, &KnowledgeSlot::TableDescription).await;

    let confirmed = confirm(&store, &id).await.unwrap();
    assert_eq!(confirmed.status, ClaimStatus::Confirmed);
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Active);

    let _ = fs::remove_dir_all(root);
}
