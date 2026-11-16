use super::*;

/// Spec Test 7: Verifies JSON serialization and deserialization for `Table`, `Column`
/// with `Exists`, `Time`, and `Numeric` requirements.
#[test]
fn test_schema_binding_serde_round_trip() {
    let cases = [
        SchemaBinding::Table,
        SchemaBinding::Column {
            column: "user_id".to_string(),
            requirement: ColumnRequirement::Exists,
        },
        SchemaBinding::Column {
            column: "created_at".to_string(),
            requirement: ColumnRequirement::Time,
        },
        SchemaBinding::Column {
            column: "total_amount".to_string(),
            requirement: ColumnRequirement::Numeric,
        },
        SchemaBinding::Columns {
            columns: vec!["customer_id".to_string(), "tenant_id".to_string()],
        },
    ];

    for case in &cases {
        let json = serde_json::to_string(case).expect("serialization must succeed");
        let deserialized: SchemaBinding =
            serde_json::from_str(&json).expect("deserialization must succeed");
        assert_eq!(case, &deserialized);
    }

    // Test explicit tagged JSON shape
    let table_json = serde_json::to_string(&SchemaBinding::Table).unwrap();
    assert_eq!(table_json, r#"{"type":"table"}"#);

    let col_time_json = serde_json::to_string(&SchemaBinding::Column {
        column: "ts".to_string(),
        requirement: ColumnRequirement::Time,
    })
    .unwrap();
    assert_eq!(
        col_time_json,
        r#"{"type":"column","column":"ts","requirement":"time"}"#
    );

    // Deserializing column without requirement field defaults to Exists
    let default_req_json = r#"{"type":"column","column":"status"}"#;
    let parsed: SchemaBinding = serde_json::from_str(default_req_json).unwrap();
    assert_eq!(
        parsed,
        SchemaBinding::Column {
            column: "status".to_string(),
            requirement: ColumnRequirement::Exists,
        }
    );
}

#[test]
fn test_column_case_insensitive_matching() {
    let binding = SchemaBinding::Column {
        column: "Return_Date".to_string(),
        requirement: ColumnRequirement::Time,
    };
    let table = make_table("rental", &[("return_date", "timestamp")]);
    assert_eq!(binding.validate(&table), BindingValidity::Valid);

    let binding_lower = SchemaBinding::Column {
        column: "return_date".to_string(),
        requirement: ColumnRequirement::Time,
    };
    let table_upper = make_table("rental", &[("RETURN_DATE", "TIMESTAMP")]);
    assert_eq!(binding_lower.validate(&table_upper), BindingValidity::Valid);
}
