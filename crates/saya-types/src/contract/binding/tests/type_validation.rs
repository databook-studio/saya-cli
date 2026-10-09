use super::*;

/// Cross-Dialect Matrix Test: Verifies temporal types across Postgres, MySQL, Snowflake, DuckDB, SQLite.
#[test]
fn test_cross_dialect_temporal_types_valid() {
    let binding = SchemaBinding::Column {
        column: "event_time".to_string(),
        requirement: ColumnRequirement::Time,
    };

    let dialects_temporal_types = [
        // Postgres
        "timestamp with time zone",
        "timestamp without time zone",
        "timestamptz",
        "date",
        "time",
        "timestamp",
        // MySQL
        "TIMESTAMP",
        "DATETIME",
        "DATE",
        "TIME",
        // Snowflake
        "TIMESTAMP_NTZ",
        "TIMESTAMP_LTZ",
        "TIMESTAMP_TZ",
        "DATE",
        "TIME",
        // DuckDB
        "TIMESTAMP",
        "DATE",
        "TIME",
        // SQLite
        "DATETIME",
        "TIMESTAMP",
    ];

    for raw_type in dialects_temporal_types {
        let table = make_table("events", &[("event_time", raw_type)]);
        assert_eq!(
            binding.validate(&table),
            BindingValidity::Valid,
            "Dialect temporal type '{raw_type}' must be Valid for Time requirement"
        );
    }
}

/// Cross-Dialect Matrix Test: Verifies numeric types across Postgres, MySQL, Snowflake, DuckDB, SQLite.
#[test]
fn test_cross_dialect_numeric_types_valid() {
    let binding = SchemaBinding::Column {
        column: "metric".to_string(),
        requirement: ColumnRequirement::Numeric,
    };

    let dialects_numeric_types = [
        // Postgres
        "integer",
        "bigint",
        "smallint",
        "bigserial",
        "serial",
        "numeric",
        "numeric(10,2)",
        "double precision",
        "real",
        // MySQL
        "INT",
        "BIGINT",
        "TINYINT",
        "DECIMAL(10,2)",
        "FLOAT",
        "DOUBLE",
        // Snowflake
        "NUMBER(38,0)",
        "FLOAT",
        "INT",
        // DuckDB
        "HUGEINT",
        "INTEGER",
        "DECIMAL(18,3)",
        "DOUBLE",
        // SQLite
        "INTEGER",
        "REAL",
    ];

    for raw_type in dialects_numeric_types {
        let table = make_table("metrics", &[("metric", raw_type)]);
        assert_eq!(
            binding.validate(&table),
            BindingValidity::Valid,
            "Dialect numeric type '{raw_type}' must be Valid for Numeric requirement"
        );
    }
}

/// Cross-Dialect Matrix Test: Verifies non-temporal types are Invalid for ColumnRequirement::Time.
#[test]
fn test_cross_dialect_non_temporal_types_invalid_for_time() {
    let binding = SchemaBinding::Column {
        column: "val".to_string(),
        requirement: ColumnRequirement::Time,
    };

    let non_temporal_types = [
        "text",
        "varchar(255)",
        "json",
        "jsonb",
        "boolean",
        "int",
        "bigint",
        "numeric(10,2)",
        "blob",
        "bytea",
        "uuid",
        "xml",
    ];

    for raw_type in non_temporal_types {
        let table = make_table("t", &[("val", raw_type)]);
        assert_eq!(
            binding.validate(&table),
            BindingValidity::Invalid,
            "Non-temporal type '{raw_type}' must be Invalid for Time requirement"
        );
    }
}
