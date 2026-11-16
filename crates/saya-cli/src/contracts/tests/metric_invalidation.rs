/// A metric whose underlying column disappears is invalidated by its binding
/// and dropped from model recall — the binding is what lets validity notice
/// the metric's columns are gone, the reason the metric binds to a column set
/// rather than to the table alone.
#[tokio::test]
async fn metric_is_invalid_when_its_underlying_column_disappears() {
    let root = temp_root("metric_drift");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::metric_definition(
            "mrr",
            "SUM(subscription_amount) WHERE status = 'active'",
            vec!["subscription_amount".into(), "status".into()],
            None,
        )
        .unwrap(),
        KnowledgeState::Active,
    )
    .await;

    // The live schema dropped `subscription_amount`, one of the metric's bound
    // columns, so the `Columns` binding reads Invalid.
    let drifted = schema_tree_for(&[(
        "orders",
        table(&[("id", "bigint", false), ("status", "text", false)]),
    )]);
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
        "a metric whose underlying column is gone is dropped for the model"
    );
    assert!(
        outcome.diagnostics.excluded_by_schema >= 1,
        "the drifted metric is counted as excluded: {:?}",
        outcome.diagnostics
    );

    // Against the unchanged schema the metric is still current and recalled.
    let current = schema_tree_for(&[(
        "orders",
        table(&[
            ("id", "bigint", false),
            ("subscription_amount", "numeric", false),
            ("status", "text", false),
        ]),
    )]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(current))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1);
    assert!(
        outcome.contracts[0]
            .claims
            .iter()
            .any(|c| matches!(c.value, ClaimPayload::MetricDefinition { .. }))
    );

    let _ = fs::remove_dir_all(root);
}

/// A join rule whose local join key disappears is invalidated by its binding;
/// a rule with no join keys binds to the table existing and stays current even
/// when an unrelated column changes.
#[tokio::test]
async fn join_rule_invalidated_by_a_dropped_key_not_by_an_unrelated_change() {
    let root = temp_root("join_drift");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::join_rule(
            "catalog.public.customers",
            vec!["customer_id".into()],
            vec!["id".into()],
            "orders.customer_id = customers.id and customers.is_active",
            None,
        )
        .unwrap(),
        KnowledgeState::Active,
    )
    .await;

    // Dropping the bound local key `customer_id` invalidates the rule.
    let dropped_key = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(dropped_key))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(
        outcome.contracts.len(),
        0,
        "a join rule whose local key is gone is dropped for the model"
    );

    // Adding an unrelated column keeps the rule current.
    let with_unrelated = schema_tree_for(&[(
        "orders",
        table(&[
            ("id", "bigint", false),
            ("customer_id", "int", false),
            ("note", "text", true),
        ]),
    )]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(with_unrelated))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1);
    assert!(
        outcome.contracts[0]
            .claims
            .iter()
            .any(|c| matches!(c.value, ClaimPayload::JoinRule { .. }))
    );

    let _ = fs::remove_dir_all(root);
}
