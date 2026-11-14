use super::*;

#[test]
fn test_numeric_requirement_validation() {
    let binding = SchemaBinding::Column {
        column: "amount".to_string(),
        requirement: ColumnRequirement::Numeric,
    };

    for valid_type in [
        "int",
        "bigint",
        "numeric(10,2)",
        "float",
        "double precision",
        "real",
        "NUMBER(38,0)",
        "serial",
    ] {
        let table = make_table("t", &[("amount", valid_type)]);
        assert_eq!(
            binding.validate(&table),
            BindingValidity::Valid,
            "type {valid_type} should be valid numeric"
        );
    }

    for invalid_type in ["varchar", "text", "boolean", "json", "date", "timestamp"] {
        let table = make_table("t", &[("amount", invalid_type)]);
        assert_eq!(
            binding.validate(&table),
            BindingValidity::Invalid,
            "type {invalid_type} should not be valid numeric"
        );
    }
}

#[test]
fn test_temporal_requirement_validation() {
    let binding = SchemaBinding::Column {
        column: "ts".to_string(),
        requirement: ColumnRequirement::Time,
    };

    for valid_type in [
        "timestamp",
        "timestamp with time zone",
        "timestamptz",
        "datetime",
        "date",
        "time",
        "TIMESTAMP_NTZ",
    ] {
        let table = make_table("t", &[("ts", valid_type)]);
        assert_eq!(
            binding.validate(&table),
            BindingValidity::Valid,
            "type {valid_type} should be valid temporal"
        );
    }

    for invalid_type in ["int", "bigint", "varchar(255)", "text", "boolean", "json"] {
        let table = make_table("t", &[("ts", invalid_type)]);
        assert_eq!(
            binding.validate(&table),
            BindingValidity::Invalid,
            "type {invalid_type} should not be valid temporal"
        );
    }
}

#[test]
fn test_validity_helpers() {
    assert!(BindingValidity::Valid.is_valid());
    assert!(!BindingValidity::Valid.is_invalid());
    assert!(!BindingValidity::Invalid.is_valid());
    assert!(BindingValidity::Invalid.is_invalid());
}
