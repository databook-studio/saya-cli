use super::*;

/// A join rule with local join keys derives a `Columns` binding on those
/// keys; a predicate-only rule with no keys derives a `Table` binding.
#[test]
fn test_derive_join_rule() {
    let keyed = ClaimPayload::join_rule(
        "catalog.public.customers",
        vec!["customer_id".into()],
        vec!["id".into()],
        "orders.customer_id = customers.id",
        None,
    )
    .unwrap();
    assert_eq!(
        SchemaBinding::derive(&KnowledgeSlot::RelationJoinRule, &keyed),
        Some(SchemaBinding::Columns {
            columns: vec!["customer_id".into()]
        })
    );
    let predicate_only = ClaimPayload::join_rule(
        "catalog.public.customers",
        vec![],
        vec![],
        "orders joins customers where customers.is_active",
        None,
    )
    .unwrap();
    assert_eq!(
        SchemaBinding::derive(&KnowledgeSlot::RelationJoinRule, &predicate_only),
        Some(SchemaBinding::Table)
    );
}

/// A metric with underlying columns derives a `Columns` binding on them; a
/// metric with no column references derives a `Table` binding.
#[test]
fn test_derive_metric_definition() {
    let with_cols = ClaimPayload::metric_definition(
        "mrr",
        "SUM(subscription_amount) WHERE status = 'active'",
        vec!["subscription_amount".into(), "status".into()],
        None,
    )
    .unwrap();
    assert_eq!(
        SchemaBinding::derive(&KnowledgeSlot::MetricDefinition, &with_cols),
        Some(SchemaBinding::Columns {
            columns: vec!["subscription_amount".into(), "status".into()]
        })
    );
    let no_cols = ClaimPayload::metric_definition("count", "COUNT(*)", vec![], None).unwrap();
    assert_eq!(
        SchemaBinding::derive(&KnowledgeSlot::MetricDefinition, &no_cols),
        Some(SchemaBinding::Table)
    );
}
