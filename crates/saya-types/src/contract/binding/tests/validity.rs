use super::*;

/// Spec Test 1 (Regression): Table with `return_date: timestamp`; binding is
/// `Column { column: "return_date", requirement: Time }`.
/// Adding an unrelated `notes: text` column to the table must leave the binding `Valid`.
#[test]
fn test_unrelated_column_addition_leaves_time_binding_valid() {
    let binding = SchemaBinding::Column {
        column: "return_date".to_string(),
        requirement: ColumnRequirement::Time,
    };

    let initial_table = make_table("rental", &[("return_date", "timestamp")]);
    assert_eq!(binding.validate(&initial_table), BindingValidity::Valid);

    let modified_table = make_table("rental", &[("return_date", "timestamp"), ("notes", "text")]);
    assert_eq!(binding.validate(&modified_table), BindingValidity::Valid);
}

/// Spec Test 2: Dropping the depended-upon column marks the binding `Invalid`.
#[test]
fn test_dropped_default_time_column_is_invalid() {
    let binding = SchemaBinding::Column {
        column: "return_date".to_string(),
        requirement: ColumnRequirement::Time,
    };

    let table_without_col = make_table("rental", &[("customer_id", "int")]);
    assert_eq!(
        binding.validate(&table_without_col),
        BindingValidity::Invalid
    );
}

/// Spec Test 3: Retyping a temporal column to `text` marks a `Time` binding `Invalid`.
#[test]
fn test_retyped_time_column_to_text_is_invalid() {
    let binding = SchemaBinding::Column {
        column: "return_date".to_string(),
        requirement: ColumnRequirement::Time,
    };

    let retyped_table = make_table("rental", &[("return_date", "text")]);
    assert_eq!(binding.validate(&retyped_table), BindingValidity::Invalid);
}

/// Spec Test 4: `SchemaBinding::Table` is unaffected by adding, removing, or retyping columns.
#[test]
fn test_table_binding_unaffected_by_column_changes() {
    let binding = SchemaBinding::Table;

    let empty_table = make_table("rental", &[]);
    assert_eq!(binding.validate(&empty_table), BindingValidity::Valid);

    let table_with_cols = make_table(
        "rental",
        &[
            ("id", "int"),
            ("rental_date", "timestamp"),
            ("notes", "text"),
        ],
    );
    assert_eq!(binding.validate(&table_with_cols), BindingValidity::Valid);

    let retyped_table = make_table("rental", &[("id", "varchar")]);
    assert_eq!(binding.validate(&retyped_table), BindingValidity::Valid);
}

/// Spec Test 5: `Column` with `Exists` requirement becomes `Invalid` only when that column
/// is dropped; dropping another unrelated column leaves it `Valid`.
#[test]
fn test_column_description_dropped_is_invalid_unrelated_drop_is_valid() {
    let binding = SchemaBinding::Column {
        column: "tier_code".to_string(),
        requirement: ColumnRequirement::Exists,
    };

    let initial_table = make_table(
        "customer",
        &[("tier_code", "varchar"), ("created_at", "timestamp")],
    );
    assert_eq!(binding.validate(&initial_table), BindingValidity::Valid);

    // Dropping unrelated column `created_at` leaves `tier_code` valid.
    let table_unrelated_dropped = make_table("customer", &[("tier_code", "varchar")]);
    assert_eq!(
        binding.validate(&table_unrelated_dropped),
        BindingValidity::Valid
    );

    // Dropping `tier_code` itself marks it invalid.
    let table_target_dropped = make_table("customer", &[("created_at", "timestamp")]);
    assert_eq!(
        binding.validate(&table_target_dropped),
        BindingValidity::Invalid
    );
}

/// Spec Test 8: `validate_table` with `None` yields `Invalid` for `Table` and `Column` bindings alike.
#[test]
fn test_missing_table_is_invalid_for_all_bindings() {
    let table_binding = SchemaBinding::Table;
    let column_binding = SchemaBinding::Column {
        column: "id".to_string(),
        requirement: ColumnRequirement::Exists,
    };
    let time_binding = SchemaBinding::Column {
        column: "created_at".to_string(),
        requirement: ColumnRequirement::Time,
    };

    assert_eq!(
        validate_table(&table_binding, None),
        BindingValidity::Invalid
    );
    assert_eq!(
        validate_table(&column_binding, None),
        BindingValidity::Invalid
    );
    assert_eq!(
        validate_table(&time_binding, None),
        BindingValidity::Invalid
    );

    let live_table = make_table("t", &[("id", "int"), ("created_at", "timestamp")]);
    assert_eq!(
        validate_table(&table_binding, Some(&live_table)),
        BindingValidity::Valid
    );
    assert_eq!(
        validate_table(&column_binding, Some(&live_table)),
        BindingValidity::Valid
    );
    assert_eq!(
        validate_table(&time_binding, Some(&live_table)),
        BindingValidity::Valid
    );
}
