#[tokio::test]
async fn join_rule_and_metric_survive_store_recall_and_render() {
    let root = temp_root("join_metric_recall");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let schema = schema_tree_for(&[(
        "orders",
        table(&[
            ("id", "bigint", false),
            ("customer_id", "int", false),
            ("subscription_amount", "numeric", false),
            ("status", "text", false),
        ]),
    )]);

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

    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1);
    let claims = &outcome.contracts[0].claims;
    assert_eq!(claims.len(), 2, "both new kinds survive recall");

    // Each new kind renders a value through the single source the prompt and
    // the receipt read, so a slot the store keeps is never silent at render.
    let join = claims
        .iter()
        .find(|c| matches!(c.value, ClaimPayload::JoinRule { .. }))
        .expect("join rule recalled");
    let (col, val) = crate::agent::recall_context::claim_value(&join.value);
    assert_eq!(col, None);
    assert_eq!(
        val,
        "orders.customer_id = customers.id and customers.is_active"
    );

    let metric = claims
        .iter()
        .find(|c| matches!(c.value, ClaimPayload::MetricDefinition { .. }))
        .expect("metric recalled");
    let (col, val) = crate::agent::recall_context::claim_value(&metric.value);
    assert_eq!(col, None);
    assert_eq!(
        val,
        "mrr = SUM(subscription_amount) WHERE status = 'active'"
    );

    let _ = fs::remove_dir_all(root);
}
