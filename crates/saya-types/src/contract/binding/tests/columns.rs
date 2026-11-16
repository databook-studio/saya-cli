use super::*;

/// A `Columns` binding is valid while every named column exists, invalid
/// the moment any one is dropped, and undisturbed by an unrelated column
/// changing — the same no-crying-wolf property the single-column binding
/// has, generalised to a set.
#[test]
fn test_columns_binding_validates_existence() {
    let binding = SchemaBinding::Columns {
        columns: vec!["customer_id".into(), "tenant_id".into()],
    };
    let table = make_table(
        "orders",
        &[
            ("customer_id", "int"),
            ("tenant_id", "int"),
            ("note", "text"),
        ],
    );
    assert_eq!(binding.validate(&table), BindingValidity::Valid);

    // Dropping an unrelated column leaves the binding valid.
    let dropped_unrelated = make_table("orders", &[("customer_id", "int"), ("tenant_id", "int")]);
    assert_eq!(binding.validate(&dropped_unrelated), BindingValidity::Valid);

    // Dropping one of the bound columns invalidates the fact.
    let dropped_one = make_table("orders", &[("customer_id", "int"), ("note", "text")]);
    assert_eq!(binding.validate(&dropped_one), BindingValidity::Invalid);

    // Retyping a bound column does not invalidate a `Columns` binding: the
    // fact depends on the column existing, not on its type, so a join key
    // widening from int to bigint stays valid.
    let retyped = make_table("orders", &[("customer_id", "bigint"), ("tenant_id", "int")]);
    assert_eq!(binding.validate(&retyped), BindingValidity::Valid);

    // Matching is case-insensitive, like the single-column binding.
    let upper = make_table("orders", &[("CUSTOMER_ID", "int"), ("TENANT_ID", "int")]);
    assert_eq!(binding.validate(&upper), BindingValidity::Valid);
}

#[test]
fn test_columns_binding_serde_round_trip() {
    let binding = SchemaBinding::Columns {
        columns: vec!["customer_id".into(), "tenant_id".into()],
    };
    let json = serde_json::to_string(&binding).unwrap();
    assert_eq!(
        json,
        r#"{"type":"columns","columns":["customer_id","tenant_id"]}"#
    );
    let back: SchemaBinding = serde_json::from_str(&json).unwrap();
    assert_eq!(back, binding);
}

/// Spec Test 6: Derived binding end-to-end validation over live tables.
#[test]
fn test_derived_binding_end_to_end_validation() {
    // 1. TableGrain -> derive -> validate
    let grain_slot = KnowledgeSlot::TableGrain;
    let grain_payload = ClaimPayload::table_grain("one row per event", None).unwrap();
    let grain_binding = SchemaBinding::derive(&grain_slot, &grain_payload).unwrap();
    let table = make_table("events", &[("id", "int")]);
    assert_eq!(grain_binding.validate(&table), BindingValidity::Valid);

    // 2. DefaultTimeColumn -> derive -> validate
    let time_slot = KnowledgeSlot::TableDefaultTime;
    let time_payload = ClaimPayload::default_time_column("event_time", None).unwrap();
    let time_binding = SchemaBinding::derive(&time_slot, &time_payload).unwrap();

    let valid_time_table = make_table("events", &[("event_time", "timestamptz")]);
    assert_eq!(
        time_binding.validate(&valid_time_table),
        BindingValidity::Valid
    );

    let invalid_time_table = make_table("events", &[("event_time", "text")]);
    assert_eq!(
        time_binding.validate(&invalid_time_table),
        BindingValidity::Invalid
    );

    let missing_time_table = make_table("events", &[("other_col", "int")]);
    assert_eq!(
        time_binding.validate(&missing_time_table),
        BindingValidity::Invalid
    );

    // 3. Measure ColumnRole -> derive -> validate
    let measure_slot = KnowledgeSlot::ColumnRole {
        column: "revenue".to_string(),
    };
    let measure_payload = ClaimPayload::column_role("revenue", ColumnRole::Measure, None).unwrap();
    let measure_binding = SchemaBinding::derive(&measure_slot, &measure_payload).unwrap();

    let valid_measure_table = make_table("sales", &[("revenue", "numeric(12,2)")]);
    assert_eq!(
        measure_binding.validate(&valid_measure_table),
        BindingValidity::Valid
    );

    let invalid_measure_table = make_table("sales", &[("revenue", "varchar(50)")]);
    assert_eq!(
        measure_binding.validate(&invalid_measure_table),
        BindingValidity::Invalid
    );
}
