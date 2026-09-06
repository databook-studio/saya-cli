//! The slot ↔ payload correspondence the repository enforces at write time.
//!
//! `value_json` reuses `ClaimPayload`'s serialisation, so the six knowledge
//! slots map one-to-one onto six payload variants. The repository refuses a
//! payload filed under the wrong slot — and a `relationship` payload, which no
//! slot names — as `CardinalityMismatch`, before anything is written. This is
//! the one place that mapping lives, so a caller cannot silently file
//! a `TableAlias` under `table.grain` and have it render as a grain.

use saya_types::{ClaimPayload, KnowledgeSlot};

/// True when `payload` is the variant `slot` names, with a matching column for
/// the column-scoped slots. `relationship` matches nothing: it has no slot.
pub(crate) fn slot_matches_payload(slot: &KnowledgeSlot, payload: &ClaimPayload) -> bool {
    match (slot, payload) {
        (KnowledgeSlot::TableDescription, ClaimPayload::TableDescription { .. })
        | (KnowledgeSlot::TableAlias, ClaimPayload::TableAlias { .. })
        | (KnowledgeSlot::TableGrain, ClaimPayload::TableGrain { .. })
        | (KnowledgeSlot::TableDefaultTime, ClaimPayload::DefaultTimeColumn { .. })
        | (KnowledgeSlot::RelationJoinRule, ClaimPayload::JoinRule { .. })
        | (KnowledgeSlot::MetricDefinition, ClaimPayload::MetricDefinition { .. }) => true,
        (
            KnowledgeSlot::ColumnDescription { column: slot_col },
            ClaimPayload::ColumnDescription { column, .. },
        ) => column == slot_col,
        (
            KnowledgeSlot::ColumnRole { column: slot_col },
            ClaimPayload::ColumnRole { column, .. },
        ) => column == slot_col,
        // Every other pairing is a mismatch: a table-level slot with a
        // column payload, a column slot with a table payload, a slot with the
        // `relationship` payload that no slot names, or a column-name disagreement.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_types::ColumnRole;

    #[test]
    fn matching_pairs_align() {
        assert!(slot_matches_payload(
            &KnowledgeSlot::TableDescription,
            &ClaimPayload::table_description("d").unwrap(),
        ));
        assert!(slot_matches_payload(
            &KnowledgeSlot::TableAlias,
            &ClaimPayload::table_alias("a").unwrap(),
        ));
        assert!(slot_matches_payload(
            &KnowledgeSlot::TableGrain,
            &ClaimPayload::table_grain("g", None).unwrap(),
        ));
        assert!(slot_matches_payload(
            &KnowledgeSlot::TableDefaultTime,
            &ClaimPayload::default_time_column("c", None).unwrap(),
        ));
        assert!(slot_matches_payload(
            &KnowledgeSlot::ColumnDescription {
                column: "created_at".into()
            },
            &ClaimPayload::column_description("created_at", "the id").unwrap(),
        ));
        assert!(slot_matches_payload(
            &KnowledgeSlot::ColumnRole {
                column: "created_at".into()
            },
            &ClaimPayload::column_role("created_at", ColumnRole::Timestamp, None).unwrap(),
        ));
    }

    #[test]
    fn wrong_variant_is_a_mismatch() {
        assert!(!slot_matches_payload(
            &KnowledgeSlot::TableGrain,
            &ClaimPayload::table_alias("a").unwrap(),
        ));
        assert!(!slot_matches_payload(
            &KnowledgeSlot::ColumnRole {
                column: "created_at".into()
            },
            &ClaimPayload::table_description("d").unwrap(),
        ));
    }

    #[test]
    fn column_name_disagreement_is_a_mismatch() {
        assert!(!slot_matches_payload(
            &KnowledgeSlot::ColumnDescription {
                column: "created_at".into()
            },
            &ClaimPayload::column_description("updated_at", "the id").unwrap(),
        ));
        assert!(!slot_matches_payload(
            &KnowledgeSlot::ColumnRole {
                column: "created_at".into()
            },
            &ClaimPayload::column_role("updated_at", ColumnRole::Timestamp, None).unwrap(),
        ));
    }

    #[test]
    fn relationship_matches_no_slot() {
        let profile = saya_types::ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
        let target = saya_types::DatabaseObjectRef::new(
            profile,
            "c",
            "s",
            "t",
            saya_types::DatabaseObjectKind::Table,
        )
        .unwrap();
        let rel = ClaimPayload::relationship(
            target,
            vec!["a".into()],
            vec!["b".into()],
            saya_types::Cardinality::OneToOne,
        )
        .unwrap();
        for slot in [
            KnowledgeSlot::TableDescription,
            KnowledgeSlot::ColumnRole { column: "c".into() },
        ] {
            assert!(
                !slot_matches_payload(&slot, &rel),
                "{slot} should not match relationship"
            );
        }
    }

    #[test]
    fn join_rule_and_metric_match_their_slots() {
        let join = ClaimPayload::join_rule(
            "catalog.public.customers",
            vec!["customer_id".into()],
            vec!["id".into()],
            "orders.customer_id = customers.id",
            None,
        )
        .unwrap();
        assert!(slot_matches_payload(
            &KnowledgeSlot::RelationJoinRule,
            &join
        ));
        let metric = ClaimPayload::metric_definition(
            "mrr",
            "SUM(subscription_amount) WHERE status = 'active'",
            vec!["subscription_amount".into()],
            None,
        )
        .unwrap();
        assert!(slot_matches_payload(
            &KnowledgeSlot::MetricDefinition,
            &metric
        ));
    }

    #[test]
    fn join_rule_and_metric_mismatch_other_slots() {
        let join = ClaimPayload::join_rule(
            "catalog.public.customers",
            vec!["customer_id".into()],
            vec!["id".into()],
            "orders.customer_id = customers.id",
            None,
        )
        .unwrap();
        assert!(!slot_matches_payload(&KnowledgeSlot::TableGrain, &join));
        assert!(!slot_matches_payload(
            &KnowledgeSlot::MetricDefinition,
            &join
        ));
        let metric = ClaimPayload::metric_definition(
            "mrr",
            "SUM(subscription_amount) WHERE status = 'active'",
            vec!["subscription_amount".into()],
            None,
        )
        .unwrap();
        assert!(!slot_matches_payload(&KnowledgeSlot::TableGrain, &metric));
        assert!(!slot_matches_payload(
            &KnowledgeSlot::RelationJoinRule,
            &metric
        ));
    }
}
