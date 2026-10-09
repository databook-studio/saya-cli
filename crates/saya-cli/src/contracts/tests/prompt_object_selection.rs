/// Spec §4.4: over-matching does not widen. A term that names no part of an
/// object must not select it. Concretely, the schema name `public` no longer
/// selects every table in the `public` schema (the old `qn.contains("public")`
/// did), and a plural term for one table does not pull in an unrelated table
/// that shares no name segment.
#[tokio::test]
async fn term_naming_no_part_of_an_object_does_not_select_it() {
    let root = temp_root("no_overmatch");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let rental = object_ref(&p, "rental");
    let customer = object_ref(&p, "customer");
    for obj in [&rental, &customer] {
        put_item(
            &store,
            obj,
            ClaimPayload::table_description("a table").unwrap(),
            KnowledgeState::Active,
        )
        .await;
    }
    // Both objects live in the `public` schema (object_ref hard-codes it).
    let schema = schema_tree_for(&[
        ("rental", table(&[("id", "bigint", false)])),
        ("customer", table(&[("id", "bigint", false)])),
    ]);

    // `public` names the schema, not either object — under the old
    // `qn.contains("public")` it matched both; it must now match neither.
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema.clone()))],
            &["public".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(
        outcome.contracts.len(),
        0,
        "schema-name term `public` must not select objects in the public schema, got {}",
        outcome.contracts.len()
    );

    // `rentals` names `rental` only — it must not also select `customer`.
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["rentals".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1);
    assert_eq!(outcome.contracts[0].object, rental);

    let _ = fs::remove_dir_all(root);
}

/// Spec §4.5: determinism — the same prompt selects the same objects in the
/// same order across runs. Selection is pure over the request and the store;
/// running the same recall twice must yield identical object sequences.
#[tokio::test]
async fn same_prompt_selects_the_same_objects_in_the_same_order() {
    let root = temp_root("selection_determinism");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    // Three objects all matched by the term `orders` (each name contains it),
    // so ranking is exercised, not just admission.
    let names = ["orders_alpha", "orders_beta", "orders_gamma"];
    for name in &names {
        let obj = object_ref(&p, name);
        put_item(
            &store,
            &obj,
            ClaimPayload::table_description("a table").unwrap(),
            KnowledgeState::Active,
        )
        .await;
    }
    let tables: Vec<(&str, Table)> = names
        .iter()
        .map(|n| (*n, table(&[("id", "bigint", false)])))
        .collect();
    let schema = schema_tree_for(&tables);
    let terms: Vec<String> = vec!["orders".to_string()];
    let bounds = RecallBounds {
        max_objects: 10,
        max_claims_per_object: 12,
        max_bytes: 16384,
    };

    // Two identical recalls against the same store — selection is pure over the
    // request, so the object sequence must be identical, not merely the set.
    let a = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema.clone()))],
            &terms,
            true,
            bounds,
        ),
    )
    .await;
    let b = recall(
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

    let names_a: Vec<String> = a
        .contracts
        .iter()
        .map(|c| c.object.object().to_string())
        .collect();
    let names_b: Vec<String> = b
        .contracts
        .iter()
        .map(|c| c.object.object().to_string())
        .collect();
    assert_eq!(
        names_a, names_b,
        "same prompt must select the same objects in the same order"
    );
    // All three matched objects are returned (under the object cap) and ranked.
    assert_eq!(names_a.len(), 3);

    let _ = fs::remove_dir_all(root);
}
