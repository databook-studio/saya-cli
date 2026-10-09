/// A confirmed `default_time_column` claim on `created_at`, made against a
/// two-column table whose live schema then drops `created_at`, so the contract
/// aggregates to `Stale` (computed, not persisted). The claim's status stays
/// `Confirmed`; only the schema drifted.
/// Seeds an `Active` `default_time_column` item on `created_at` for `object`
/// against a *drifted* schema that dropped `created_at`, so the item reads
/// `Invalid` (the D-4 "stale" analog: `Active` state, `Invalid` computed
/// validity). Returns the item's `ki-…` id. The schema is cached in the store
/// so `show` classifies against it.
async fn seed_active_time_item_drifted(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
) -> ClaimId {
    let id = seed_active_time_item(store, object, "created_at").await;
    let drifted = schema_tree_for(&[(object.object(), table(&[("id", "bigint", false)]))]);
    store
        .upsert_schema(object.profile().as_str(), &drifted)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn show_keeps_a_stale_contract_with_state_and_claims_for_a_human() {
    let root = temp_root("show_stale_human");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let id = seed_active_time_item_drifted(&store, &obj).await;

    // The human-review path: `show` keeps the stale contract, its `Stale`
    // state, and the claim itself so a reviewer can act on it. The model path
    // (`ForModel`) is what drops; `contracts show` is `ForHumanReview`.
    let shown = show(
        &store,
        &obj,
        &avail(schema_tree_for(&[(
            "orders",
            table(&[("id", "bigint", false)]),
        )])),
        RetrievalPolicy::ForHumanReview,
        FRESH_NOW,
    )
    .await
    .unwrap()
    .expect("a stale contract is shown to a human, not hidden");
    assert_eq!(
        shown.schema_state,
        ContractSchemaState::Stale,
        "the state is reported as stale"
    );
    let ids: Vec<ClaimId> = shown.claims.iter().map(|c| c.id.clone()).collect();
    assert!(
        ids.contains(&id),
        "the stale item is kept for a human reviewer, got {ids:?}"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn show_for_model_drops_a_stale_contracts_claims_but_names_the_object() {
    let root = temp_root("show_stale_model");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    seed_active_time_item_drifted(&store, &obj).await;

    // The model-facing path: `contract_read` uses `ForModel`. The object is
    // still named and reported `Stale`, but the gone-column claim is not handed
    // to the model as a current fact.
    let shown = show(
        &store,
        &obj,
        &avail(schema_tree_for(&[(
            "orders",
            table(&[("id", "bigint", false)]),
        )])),
        RetrievalPolicy::ForModel,
        FRESH_NOW,
    )
    .await
    .unwrap()
    .expect("the stale object is reported, not hidden");
    assert_eq!(shown.schema_state, ContractSchemaState::Stale);
    assert!(
        shown.claims.is_empty(),
        "no claims to act on for a stale object, got {}",
        shown.claims.len()
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn recall_for_model_counts_a_stale_exclusion() {
    let root = temp_root("recall_stale_count");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    // A confirmed `default_time_column` on `created_at`. Its D-4 binding is
    // `Column { created_at, Time }`.
    put_item(
        &store,
        &obj,
        ClaimPayload::default_time_column("created_at", None).unwrap(),
        KnowledgeState::Active,
    )
    .await;

    // The model-facing recall path drops the contract whose binding the live
    // schema no longer satisfies (the bound column `created_at` is gone) and
    // counts it in `excluded_by_schema`, so a user can see why a fact they
    // remembered stopped appearing — the exclusion is not silent.
    let drifted = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(drifted))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(
        outcome.contracts.len(),
        0,
        "the stale contract is dropped for the model"
    );
    assert!(
        outcome.diagnostics.excluded_by_schema >= 1,
        "the stale exclusion is counted: {:?}",
        outcome.diagnostics
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Bulk store reads — recall over many objects.
//
// The N+1 fix moved recall and the queue from one store round trip per object
// to a bounded number per profile. What remains here is the property the bulk
// path was named for: recall over several objects is correct under it. The
// reconcile-atomicity test that lived here was deleted in Chunk 5 with the
// reconciler itself (staleness is computed at read time, never persisted).
// ---------------------------------------------------------------------------

/// Recall over many objects of one profile returns every match — the bulk
/// `knowledge_for_profile` path groups items across objects without a query
/// per object, and selection still ranks and admits them as the per-object loop
/// did. Twenty objects is well under the object cap, so all reach selection.
#[tokio::test]
async fn recall_over_many_objects_returns_every_match() {
    let root = temp_root("recall_many_objects");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let names: Vec<String> = (0..20).map(|i| format!("orders{i}")).collect();
    for name in &names {
        let obj = object_ref(&p, name);
        put_item(
            &store,
            &obj,
            ClaimPayload::table_alias(name.as_str()).unwrap(),
            KnowledgeState::Active,
        )
        .await;
    }

    // The live schema names all twenty so each reads `current` — staleness is
    // not what this test exercises, and a stale match would be dropped by the
    // model-facing policy before the object bound applied, contaminating the
    // count. A term that matches every object by name selects all twenty.
    let tables: Vec<(&str, Table)> = names
        .iter()
        .map(|n| (n.as_str(), table(&[("id", "bigint", false)])))
        .collect();
    let schema = schema_tree_for(&tables);
    let terms: Vec<String> = vec!["orders".to_string()];
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &terms,
            true,
            RecallBounds {
                max_objects: 20,
                max_claims_per_object: 12,
                max_bytes: 16384,
            },
        ),
    )
    .await;
    assert_eq!(
        outcome.contracts.len(),
        20,
        "bulk recall must return every matching object, got {}",
        outcome.contracts.len()
    );
    let selected: Vec<String> = outcome
        .contracts
        .iter()
        .map(|c| c.object.object().to_string())
        .collect();
    for name in &names {
        assert!(selected.contains(name), "missing {name} in {selected:?}");
    }

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
