#[test]
fn item_validity_for_a_satisfied_binding_is_valid() {
    let item = time_column_item(KnowledgeState::Active, &time_binding());
    let schema = schema_with(table(&[("created_at", "timestamp", false)]));
    assert_eq!(
        item_validity_for(
            &item,
            &SchemaAvailability::available(schema, FRESH_NOW),
            SchemaFreshness::for_model(FRESH_NOW),
        ),
        KnowledgeValidity::Valid,
        "a binding the live schema satisfies is valid"
    );
}

// 2. An unrelated column added to the table does NOT invalidate the
//    binding (D-4's whole point). Under the old whole-table fingerprint
//    model this read NeedsReview; under D-4 it reads Valid.
#[test]
fn item_validity_unrelated_column_change_is_valid_under_d4() {
    let item = time_column_item(KnowledgeState::Active, &time_binding());
    let schema = schema_with(table(&[
        ("id", "bigint", false),
        ("created_at", "timestamp", false),
        ("note", "text", true),
    ]));
    assert_eq!(
        item_validity_for(
            &item,
            &SchemaAvailability::available(schema, FRESH_NOW),
            SchemaFreshness::for_model(FRESH_NOW),
        ),
        KnowledgeValidity::Valid,
        "an unrelated column changing must not invalidate under D-4"
    );
}

// 3. A dropped referenced column → Invalid (the gone-column case, D-4).
#[test]
fn item_validity_dropped_bound_column_is_invalid() {
    let item = time_column_item(KnowledgeState::Active, &time_binding());
    let schema = schema_with(table(&[("id", "bigint", false)]));
    assert_eq!(
        item_validity_for(
            &item,
            &SchemaAvailability::available(schema, FRESH_NOW),
            SchemaFreshness::for_model(FRESH_NOW),
        ),
        KnowledgeValidity::Invalid,
        "a gone bound column makes the item invalid"
    );
}

// 4. A bound column retyped away from temporal → Invalid.
#[test]
fn item_validity_retyped_bound_column_is_invalid() {
    let item = time_column_item(KnowledgeState::Active, &time_binding());
    let schema = schema_with(table(&[("created_at", "text", false)]));
    assert_eq!(
        item_validity_for(
            &item,
            &SchemaAvailability::available(schema, FRESH_NOW),
            SchemaFreshness::for_model(FRESH_NOW),
        ),
        KnowledgeValidity::Invalid,
        "a bound column that lost its temporal type is invalid"
    );
}

// 5. A missing or unavailable schema → SchemaUnavailable, never Invalid.
#[test]
fn item_validity_missing_or_unavailable_schema_is_schema_unavailable() {
    let item = time_column_item(KnowledgeState::Active, &time_binding());
    assert_eq!(
        item_validity_for(
            &item,
            &SchemaAvailability::Missing,
            SchemaFreshness::for_model(FRESH_NOW),
        ),
        KnowledgeValidity::SchemaUnavailable,
        "a missing cache is not evidence a bound column is gone"
    );
    assert_eq!(
        item_validity_for(
            &item,
            &SchemaAvailability::Unavailable,
            SchemaFreshness::for_model(FRESH_NOW),
        ),
        KnowledgeValidity::SchemaUnavailable,
        "a store error is not evidence a bound column is gone"
    );
}

// 6. A stale-by-age cached schema beyond the model-path freshness bound →
//    SchemaUnavailable (kept, labelled), not Valid.
#[test]
fn item_validity_stale_by_age_cache_is_schema_unavailable() {
    let item = time_column_item(KnowledgeState::Active, &time_binding());
    let schema = schema_with(table(&[("created_at", "timestamp", false)]));
    let now = MODEL_SCHEMA_MAX_AGE_MS + 1;
    assert_eq!(
        item_validity_for(
            &item,
            &SchemaAvailability::available(schema, 0),
            SchemaFreshness::for_model(now),
        ),
        KnowledgeValidity::SchemaUnavailable,
        "a stale-by-age cache cannot vouch for currency on the model path"
    );
}

// 7. A non-current fingerprint version → NeedsReview: the binding was
//    derived under a format this build cannot faithfully compare, so the
//    item is held for human review rather than trusted or silently dropped.
#[test]
fn item_validity_non_current_fingerprint_version_is_needs_review() {
    let mut item = time_column_item(KnowledgeState::Active, &time_binding());
    item.fingerprint_version = FINGERPRINT_VERSION + 1;
    let schema = schema_with(table(&[("created_at", "timestamp", false)]));
    assert_eq!(
        item_validity_for(
            &item,
            &SchemaAvailability::available(schema, FRESH_NOW),
            SchemaFreshness::for_model(FRESH_NOW),
        ),
        KnowledgeValidity::NeedsReview,
        "a binding written under an unknown format is held for review"
    );
}

// 8. A Dismissed item is Invalid without consulting the schema, and stays
//    Invalid when the schema is missing too.
#[test]
fn item_validity_dismissed_is_invalid_regardless_of_schema() {
    let item = time_column_item(KnowledgeState::Dismissed, &time_binding());
    let schema = schema_with(table(&[("created_at", "timestamp", false)]));
    assert_eq!(
        item_validity_for(
            &item,
            &SchemaAvailability::available(schema, FRESH_NOW),
            SchemaFreshness::for_model(FRESH_NOW),
        ),
        KnowledgeValidity::Invalid,
        "a dismissed item is not a live fact"
    );
    assert_eq!(
        item_validity_for(
            &item,
            &SchemaAvailability::Missing,
            SchemaFreshness::for_model(FRESH_NOW),
        ),
        KnowledgeValidity::Invalid,
        "dismissal beats schema unavailability"
    );
}

// 9. A Table binding classifies Valid whenever the table exists, and
//    Invalid when it is gone — independent of any column.
#[test]
fn item_validity_table_binding_depends_only_on_the_table_existing() {
    let item = KnowledgeItem {
        id: "ki-2".into(),
        object: item_object(),
        slot: KnowledgeSlot::TableGrain,
        cardinality_single: true,
        value: saya_types::ClaimPayload::table_grain("one row per order", None).unwrap(),
        source: ClaimOrigin::UserExplicit,
        state: KnowledgeState::Active,
        cleanup: saya_store::CleanupState::Complete,
        schema_binding_json: table_binding(),
        fingerprint_version: FINGERPRINT_VERSION,
        created_unix_ms: 0,
        updated_unix_ms: 0,
    };
    let present = schema_with(table(&[("id", "bigint", false)]));
    assert_eq!(
        item_validity_for(
            &item,
            &SchemaAvailability::available(present, FRESH_NOW),
            SchemaFreshness::for_model(FRESH_NOW),
        ),
        KnowledgeValidity::Valid,
        "a table binding is valid when the table exists"
    );
    let absent_table = schema_with(saya_types::Table {
        name: "other".into(),
        primary_key: vec![],
        foreign_keys: vec![],
        columns: vec![Column {
            name: "id".into(),
            data_type: "bigint".into(),
            nullable: false,
        }],
    });
    assert_eq!(
        item_validity_for(
            &item,
            &SchemaAvailability::available(absent_table, FRESH_NOW),
            SchemaFreshness::for_model(FRESH_NOW),
        ),
        KnowledgeValidity::Invalid,
        "a table binding is invalid when the table is gone"
    );
}

// 10. A binding JSON this build cannot deserialize (a row written by an
//     incompatible build) is held for review, not silently dropped.
#[test]
fn item_validity_unparseable_binding_is_needs_review() {
    let item = time_column_item(KnowledgeState::Active, "not json at all");
    let schema = schema_with(table(&[("created_at", "timestamp", false)]));
    assert_eq!(
        item_validity_for(
            &item,
            &SchemaAvailability::available(schema, FRESH_NOW),
            SchemaFreshness::for_model(FRESH_NOW),
        ),
        KnowledgeValidity::NeedsReview,
        "an uninterpretable binding is held for human review, not dropped"
    );
}
