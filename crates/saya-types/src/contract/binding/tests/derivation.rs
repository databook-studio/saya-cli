use super::*;

/// Table-scoped knowledge derives a table-existence binding.
#[test]
fn test_derive_table_slots() {
    let desc_slot = KnowledgeSlot::TableDescription;
    let desc_payload = ClaimPayload::table_description("A table of rentals").unwrap();
    assert_eq!(
        SchemaBinding::derive(&desc_slot, &desc_payload),
        Some(SchemaBinding::Table)
    );

    let alias_slot = KnowledgeSlot::TableAlias;
    let alias_payload = ClaimPayload::table_alias("rentals").unwrap();
    assert_eq!(
        SchemaBinding::derive(&alias_slot, &alias_payload),
        Some(SchemaBinding::Table)
    );

    let grain_slot = KnowledgeSlot::TableGrain;
    let grain_payload = ClaimPayload::table_grain("one row per rental event", None).unwrap();
    assert_eq!(
        SchemaBinding::derive(&grain_slot, &grain_payload),
        Some(SchemaBinding::Table)
    );

    let note_slot = KnowledgeSlot::TableUserNote;
    let note_payload = ClaimPayload::table_user_note("Keep refunds separate.").unwrap();
    assert_eq!(
        SchemaBinding::derive(&note_slot, &note_payload),
        Some(SchemaBinding::Table)
    );
}

/// Chunk 2 test: TableDefaultTime derives SchemaBinding::Column with ColumnRequirement::Time.
#[test]
fn test_derive_default_time() {
    let slot = KnowledgeSlot::TableDefaultTime;
    let payload = ClaimPayload::default_time_column("return_date", None).unwrap();
    assert_eq!(
        SchemaBinding::derive(&slot, &payload),
        Some(SchemaBinding::Column {
            column: "return_date".to_string(),
            requirement: ColumnRequirement::Time,
        })
    );
}

/// Chunk 2 test: ColumnDescription derives SchemaBinding::Column with ColumnRequirement::Exists.
#[test]
fn test_derive_column_description() {
    let slot = KnowledgeSlot::ColumnDescription {
        column: "tier_code".to_string(),
    };
    let payload = ClaimPayload::column_description("tier_code", "Loyalty tier").unwrap();
    assert_eq!(
        SchemaBinding::derive(&slot, &payload),
        Some(SchemaBinding::Column {
            column: "tier_code".to_string(),
            requirement: ColumnRequirement::Exists,
        })
    );
}

/// Chunk 2 test: ColumnRole matrix derivations for all role variants.
#[test]
fn test_derive_column_role_matrix() {
    // Timestamp -> Time
    let ts_slot = KnowledgeSlot::ColumnRole {
        column: "created_at".to_string(),
    };
    let ts_payload = ClaimPayload::column_role("created_at", ColumnRole::Timestamp, None).unwrap();
    assert_eq!(
        SchemaBinding::derive(&ts_slot, &ts_payload),
        Some(SchemaBinding::Column {
            column: "created_at".to_string(),
            requirement: ColumnRequirement::Time,
        })
    );

    // Measure -> Numeric
    let measure_slot = KnowledgeSlot::ColumnRole {
        column: "amount".to_string(),
    };
    let measure_payload = ClaimPayload::column_role("amount", ColumnRole::Measure, None).unwrap();
    assert_eq!(
        SchemaBinding::derive(&measure_slot, &measure_payload),
        Some(SchemaBinding::Column {
            column: "amount".to_string(),
            requirement: ColumnRequirement::Numeric,
        })
    );

    // Identifier, Dimension, Sensitive -> Exists
    for (col, role) in [
        ("user_id", ColumnRole::Identifier),
        ("category", ColumnRole::Dimension),
        ("ssn", ColumnRole::Sensitive),
    ] {
        let role_slot = KnowledgeSlot::ColumnRole {
            column: col.to_string(),
        };
        let role_payload = ClaimPayload::column_role(col, role, None).unwrap();
        assert_eq!(
            SchemaBinding::derive(&role_slot, &role_payload),
            Some(SchemaBinding::Column {
                column: col.to_string(),
                requirement: ColumnRequirement::Exists,
            }),
            "Role {role:?} must derive ColumnRequirement::Exists"
        );
    }
}

/// Chunk 2 test: Slot/payload kind mismatch returns None.
#[test]
fn test_derive_slot_payload_mismatch_returns_none() {
    let grain_slot = KnowledgeSlot::TableGrain;
    let alias_payload = ClaimPayload::table_alias("rentals").unwrap();
    assert_eq!(SchemaBinding::derive(&grain_slot, &alias_payload), None);

    let desc_slot = KnowledgeSlot::TableDescription;
    let default_time_payload = ClaimPayload::default_time_column("created_at", None).unwrap();
    assert_eq!(
        SchemaBinding::derive(&desc_slot, &default_time_payload),
        None
    );

    let col_desc_slot = KnowledgeSlot::ColumnDescription {
        column: "tier".to_string(),
    };
    let col_role_payload = ClaimPayload::column_role("tier", ColumnRole::Identifier, None).unwrap();
    assert_eq!(
        SchemaBinding::derive(&col_desc_slot, &col_role_payload),
        None
    );
}

/// Chunk 2 test: Column name disagreement between slot and payload returns None.
#[test]
fn test_derive_column_name_mismatch_returns_none() {
    let role_slot = KnowledgeSlot::ColumnRole {
        column: "column_a".to_string(),
    };
    let role_payload = ClaimPayload::column_role("column_b", ColumnRole::Timestamp, None).unwrap();
    assert_eq!(SchemaBinding::derive(&role_slot, &role_payload), None);

    let desc_slot = KnowledgeSlot::ColumnDescription {
        column: "column_a".to_string(),
    };
    let desc_payload = ClaimPayload::column_description("column_b", "desc").unwrap();
    assert_eq!(SchemaBinding::derive(&desc_slot, &desc_payload), None);
}

/// Chunk 2 test: ClaimPayload::Relationship has no corresponding slot and returns None.
#[test]
fn test_derive_relationship_returns_none() {
    let target = make_target();
    let relationship_payload = ClaimPayload::relationship(
        target,
        vec!["customer_id".into()],
        vec!["id".into()],
        Cardinality::ManyToOne,
    )
    .unwrap();

    let slots = [
        KnowledgeSlot::TableDescription,
        KnowledgeSlot::TableAlias,
        KnowledgeSlot::TableGrain,
        KnowledgeSlot::TableDefaultTime,
        KnowledgeSlot::ColumnDescription {
            column: "customer_id".into(),
        },
        KnowledgeSlot::ColumnRole {
            column: "customer_id".into(),
        },
    ];

    for slot in &slots {
        assert_eq!(
            SchemaBinding::derive(slot, &relationship_payload),
            None,
            "Relationship payload must not derive a binding for slot {slot:?}"
        );
    }
}
