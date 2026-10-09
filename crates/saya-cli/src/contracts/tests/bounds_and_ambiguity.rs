#[tokio::test]
async fn bounds_hold_on_objects_and_claims() {
    let root = temp_root("bounds");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let objs: Vec<DatabaseObjectRef> = (0..3)
        .map(|i| object_ref(&p, &format!("orders{i}")))
        .collect();
    for obj in &objs {
        put_item(
            &store,
            obj,
            ClaimPayload::table_alias(obj.object()).unwrap(),
            KnowledgeState::Active,
        )
        .await;
    }
    // D-3 NOTE: `table_alias` is multi-valued (max 4). The per-object bound test
    // needs more items than `max_claims_per_object` (3) on one object, so seed 4
    // aliases — the most one multi-valued slot admits — and let the cap of 3
    // truncate the tail. (The old test seeded 20 of the same slot, which D-3
    // cardinality refuses past 4.)
    let heavy = object_ref(&p, "heavy");
    for i in 0..4 {
        put_item(
            &store,
            &heavy,
            ClaimPayload::table_alias(format!("a{i}")).unwrap(),
            KnowledgeState::Active,
        )
        .await;
    }

    // max_objects = 2 over 3 matches -> exactly 2, truncated. All three orders
    // objects are present in the live schema so each reads `current` -- this
    // test is about bounds, not staleness, and a stale match would be dropped
    // by the model-facing policy before the object bound applied,
    // contaminating the count.
    let terms: Vec<String> = objs.iter().map(|o| o.object().to_string()).collect();
    let bounds = RecallBounds {
        max_objects: 2,
        max_claims_per_object: 12,
        max_bytes: 16384,
    };
    let schema = schema_tree_for(&[
        ("orders0", table(&[("id", "bigint", false)])),
        ("orders1", table(&[("id", "bigint", false)])),
        ("orders2", table(&[("id", "bigint", false)])),
    ]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &terms,
            true,
            bounds,
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 2, "max_objects not honored");
    assert!(
        outcome.contracts.iter().any(|c| c.truncated),
        "object truncation not flagged"
    );

    // max_claims_per_object = 3 on `heavy` (20 aliases) -> 3, truncated.
    let bounds2 = RecallBounds {
        max_objects: 5,
        max_claims_per_object: 3,
        max_bytes: 16384,
    };
    let schema_h = schema_tree_for(&[("heavy", table(&[("id", "bigint", false)]))]);
    let outcome2 = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema_h.clone()))],
            &["heavy".to_string()],
            true,
            bounds2,
        ),
    )
    .await;
    assert_eq!(outcome2.contracts.len(), 1);
    assert_eq!(outcome2.contracts[0].claims.len(), 3);
    assert!(outcome2.contracts[0].truncated);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 4: ambiguous alias returns every match
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ambiguous_alias_returns_every_match() {
    let root = temp_root("ambiguous");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj1 = object_ref(&p, "table_one");
    let obj2 = object_ref(&p, "table_two");
    put_item(
        &store,
        &obj1,
        ClaimPayload::table_alias("shared").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    put_item(
        &store,
        &obj2,
        ClaimPayload::table_alias("shared").unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let schema = schema_tree_for(&[
        ("table_one", table(&[("id", "bigint", false)])),
        ("table_two", table(&[("id", "bigint", false)])),
    ]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["shared".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(
        outcome.contracts.len(),
        2,
        "ambiguous alias should return both"
    );
    let names: Vec<&str> = outcome
        .contracts
        .iter()
        .map(|c| c.object.object())
        .collect();
    assert!(names.contains(&"table_one"));
    assert!(names.contains(&"table_two"));

    let _ = fs::remove_dir_all(root);
}
