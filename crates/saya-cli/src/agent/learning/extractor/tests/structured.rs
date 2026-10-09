use super::*;

#[test]
fn test_parse_builds_a_join_rule_from_structured_fields() {
    // A join rule arrives as the condition in `value` plus the target and
    // the paired join keys on the optional fields; the parser assembles
    // them into one `JoinRule` payload filed against the local object.
    let table = setup_test_table();
    let json = r#"{
            "proposals": [
                {
                    "object_id": "T0",
                    "slot": "relation.join_rule",
                    "value": "orders.customer_id = customers.id, and only where customers.is_active",
                    "target": "analytics.public.customers",
                    "local_columns": ["customer_id"],
                    "target_columns": ["id"],
                    "reason": "only active customers count toward an order",
                    "origin": "user_explicit",
                    "confidence": 1.0
                }
            ]
        }"#;
    let res = parse_extraction_response(json, &table).unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].slot, KnowledgeSlot::RelationJoinRule);
    assert_eq!(
        res[0].value,
        ClaimPayload::join_rule(
            "analytics.public.customers",
            vec!["customer_id".into()],
            vec!["id".into()],
            "orders.customer_id = customers.id, and only where customers.is_active",
            Some("only active customers count toward an order"),
        )
        .unwrap()
    );
}

#[test]
fn test_parse_builds_a_metric_definition_from_structured_fields() {
    let table = setup_test_table();
    let json = r#"{
            "proposals": [
                {
                    "object_id": "T0",
                    "slot": "metric.definition",
                    "name": "mrr",
                    "value": "SUM(subscription_amount) WHERE status = 'active'",
                    "columns": ["subscription_amount", "status"],
                    "reason": "recurring revenue only",
                    "origin": "assistant_inferred",
                    "confidence": 0.9
                }
            ]
        }"#;
    let res = parse_extraction_response(json, &table).unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].slot, KnowledgeSlot::MetricDefinition);
    assert_eq!(
        res[0].value,
        ClaimPayload::metric_definition(
            "mrr",
            "SUM(subscription_amount) WHERE status = 'active'",
            vec!["subscription_amount".into(), "status".into()],
            Some("recurring revenue only"),
        )
        .unwrap()
    );
}

#[test]
fn test_parse_drops_a_join_rule_without_a_target() {
    // A join rule with no target names no relation; the proposal is dropped
    // rather than filed as a half-formed fact.
    let table = setup_test_table();
    let json = r#"{
            "proposals": [
                {
                    "object_id": "T0",
                    "slot": "relation.join_rule",
                    "value": "orders joins customers on is_active",
                    "origin": "assistant_inferred"
                }
            ]
        }"#;
    let res = parse_extraction_response(json, &table).unwrap();
    assert!(
        res.is_empty(),
        "a join rule without a target must be dropped"
    );
}

#[test]
fn test_parse_drops_a_metric_definition_without_a_name() {
    let table = setup_test_table();
    let json = r#"{
            "proposals": [
                {
                    "object_id": "T0",
                    "slot": "metric.definition",
                    "value": "SUM(subscription_amount) WHERE status = 'active'",
                    "columns": ["subscription_amount"],
                    "origin": "assistant_inferred"
                }
            ]
        }"#;
    let res = parse_extraction_response(json, &table).unwrap();
    assert!(
        res.is_empty(),
        "a metric definition without a name must be dropped"
    );
}

#[test]
fn test_parse_rejects_secret_or_credential_values() {
    let table = setup_test_table();
    let json = r#"{
            "proposals": [
                {
                    "object_id": "T0",
                    "slot": "table.description",
                    "value": "Bearer sk-1234567890abcdef",
                    "origin": "assistant_inferred"
                },
                {
                    "object_id": "T0",
                    "slot": "table.description",
                    "value": "Valid table description",
                    "origin": "assistant_inferred"
                }
            ]
        }"#;

    let res = parse_extraction_response(json, &table).unwrap();
    assert_eq!(res.len(), 1, "Credential proposal must be rejected");
    assert_eq!(
        res[0].value,
        ClaimPayload::table_description("Valid table description").unwrap()
    );
}
