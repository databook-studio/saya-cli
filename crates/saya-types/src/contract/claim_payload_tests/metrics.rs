use super::*;

#[test]
fn metric_definition_rejects_empty_name_and_definition() {
    assert!(ClaimPayload::metric_definition("", "SUM(amount)", vec![], None).is_err());
    assert!(ClaimPayload::metric_definition("mrr", "", vec![], None).is_err());
    assert!(ClaimPayload::metric_definition("mrr", "SUM(amount)\n", vec![], None).is_err());
}

#[test]
fn metric_definition_bounds_and_validates_columns() {
    assert!(ClaimPayload::metric_definition("mrr", "SUM(amount)", vec!["".into()], None).is_err());
    let many: Vec<String> = (0..=MAX_REFERENCED_COLUMNS)
        .map(|i| format!("c{i}"))
        .collect();
    assert_eq!(
        ClaimPayload::metric_definition("mrr", "SUM(amount)", many, None).unwrap_err(),
        ContractError::TooManyColumns
    );
    // An empty column list is allowed: a metric with no column references
    // binds to the table existing.
    assert!(ClaimPayload::metric_definition("count", "COUNT(*)", vec![], None).is_ok());
}

#[test]
fn metric_definition_carries_an_optional_reason() {
    let none = ClaimPayload::metric_definition(
        "mrr",
        "SUM(subscription_amount) WHERE status = 'active'",
        vec!["subscription_amount".into()],
        None,
    )
    .unwrap();
    assert!(matches!(
        none,
        ClaimPayload::MetricDefinition { reason: None, .. }
    ));
    let with = ClaimPayload::metric_definition(
        "mrr",
        "SUM(subscription_amount) WHERE status = 'active'",
        vec!["subscription_amount".into()],
        Some("recurring revenue only"),
    )
    .unwrap();
    assert!(matches!(
        with,
        ClaimPayload::MetricDefinition { reason: Some(r), .. } if r == "recurring revenue only"
    ));
}

/// A join rule or metric written before the reason field exists must decode
/// with `reason: None`, not fail — a decode failure on the read path would
/// take out a user's entire memory. The JSON is hand-written to prove the
/// old shape loads, not just that the current serializer round-trips.
#[test]
fn old_join_rule_and_metric_without_reason_decode_as_none() {
    let old_join = r#"{"kind":"join_rule","target":"catalog.public.customers","local_columns":["customer_id"],"target_columns":["id"],"condition":"orders.customer_id = customers.id"}"#;
    let p: ClaimPayload = serde_json::from_str(old_join).unwrap();
    assert!(matches!(
        p,
        ClaimPayload::JoinRule { ref target, ref condition, ref reason, .. }
            if target == "catalog.public.customers"
                && condition == "orders.customer_id = customers.id"
                && reason.is_none()
    ));

    let old_metric = r#"{"kind":"metric_definition","name":"mrr","definition":"SUM(subscription_amount) WHERE status = 'active'","columns":["subscription_amount","status"]}"#;
    let p: ClaimPayload = serde_json::from_str(old_metric).unwrap();
    assert!(matches!(
        p,
        ClaimPayload::MetricDefinition { ref name, ref definition, ref columns, ref reason }
            if name == "mrr"
                && definition == "SUM(subscription_amount) WHERE status = 'active'"
                && columns.as_slice() == ["subscription_amount", "status"]
                && reason.is_none()
    ));
}

#[test]
fn join_rule_and_metric_serde_round_trip() {
    let join = ClaimPayload::join_rule(
        "catalog.public.customers",
        vec!["customer_id".into()],
        vec!["id".into()],
        "orders.customer_id = customers.id and customers.is_active",
        Some("only active customers count"),
    )
    .unwrap();
    let json = serde_json::to_string(&join).unwrap();
    let back: ClaimPayload = serde_json::from_str(&json).unwrap();
    assert_eq!(join, back);

    let metric = ClaimPayload::metric_definition(
        "mrr",
        "SUM(subscription_amount) WHERE status = 'active'",
        vec!["subscription_amount".into(), "status".into()],
        Some("recurring revenue only"),
    )
    .unwrap();
    let json = serde_json::to_string(&metric).unwrap();
    let back: ClaimPayload = serde_json::from_str(&json).unwrap();
    assert_eq!(metric, back);
}
