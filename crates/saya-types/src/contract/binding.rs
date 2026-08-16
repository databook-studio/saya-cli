use serde::{Deserialize, Serialize};

use crate::schema::Table;

/// Verdict of validating a knowledge item's schema binding against live table metadata.
///
/// This is strictly a binary verdict: either all schema dependencies required
/// by the business fact exist and satisfy semantic type expectations (`Valid`),
/// or a required dependency is missing or contradicted (`Invalid`).
///
/// Prior whole-table fingerprint systems used a three-way verdict with `NeedsReview`
/// whenever unreferenced columns changed, which caused false alarms ("crying wolf")
/// and eroded trust in persistent knowledge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingValidity {
    Valid,
    Invalid,
}

impl BindingValidity {
    /// True when the binding satisfies all live schema dependencies.
    pub const fn is_valid(self) -> bool {
        matches!(self, Self::Valid)
    }

    /// True when any live schema dependency is missing or contradicted.
    pub const fn is_invalid(self) -> bool {
        matches!(self, Self::Invalid)
    }
}

/// Semantic type requirement placed on a depended-upon column.
///
/// Rather than snapshotting exact connector type strings (which breaks on
/// harmless type widenings like `VARCHAR(20)` -> `VARCHAR(50)` or `INT` -> `BIGINT`),
/// bindings record the semantic capability the fact requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ColumnRequirement {
    /// The column must exist in the live table, with any data type.
    #[default]
    Exists,
    /// The column must exist and possess a temporal type (date, time, timestamp).
    Time,
    /// The column must exist and possess a numeric type (int, float, decimal, etc.).
    Numeric,
}

/// Structural dependencies a piece of knowledge requires from the database schema.
///
/// A fact depends only on what it actually uses:
/// - Table-level knowledge (`TableDescription`, `TableAlias`, `TableGrain`) depends
///   only on the table existing.
/// - Column-scoped knowledge (`ColumnDescription`, `ColumnRole`, `TableDefaultTime`)
///   depends on the specific named column existing and optionally satisfying a
///   semantic type constraint ([`ColumnRequirement`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SchemaBinding {
    /// Depends only on the table existing in the schema.
    Table,
    /// Depends on a named column existing and satisfying `requirement`.
    Column {
        column: String,
        #[serde(default)]
        requirement: ColumnRequirement,
    },
}

impl SchemaBinding {
    /// Validates this binding against a live table definition.
    ///
    /// Evaluates whether the referenced table and column dependencies are
    /// satisfied. Matching column names is case-insensitive to align with SQL
    /// catalog conventions across database dialects.
    pub fn validate(&self, table: &Table) -> BindingValidity {
        match self {
            Self::Table => BindingValidity::Valid,
            Self::Column {
                column,
                requirement,
            } => {
                let Some(col) = table
                    .columns
                    .iter()
                    .find(|c| c.name.eq_ignore_ascii_case(column))
                else {
                    return BindingValidity::Invalid;
                };

                match requirement {
                    ColumnRequirement::Exists => BindingValidity::Valid,
                    ColumnRequirement::Time => {
                        if is_temporal_type(&col.data_type) {
                            BindingValidity::Valid
                        } else {
                            BindingValidity::Invalid
                        }
                    }
                    ColumnRequirement::Numeric => {
                        if is_numeric_type(&col.data_type) {
                            BindingValidity::Valid
                        } else {
                            BindingValidity::Invalid
                        }
                    }
                }
            }
        }
    }
}

/// Validates a schema binding against an optional live table reference.
///
/// If the table does not exist (`None`), any binding is `Invalid`.
/// If the table exists (`Some(table)`), delegates to [`SchemaBinding::validate`].
pub fn validate_table(binding: &SchemaBinding, table: Option<&Table>) -> BindingValidity {
    match table {
        Some(t) => binding.validate(t),
        None => BindingValidity::Invalid,
    }
}

/// Classifies whether a connector's raw data type string represents a temporal
/// value (date, time, timestamp, or datetime).
///
/// We classify types by semantic substring matching rather than exact type enum
/// matching because database dialects express temporal representations with
/// varied spellings, precision parameters, and timezone suffixes (e.g. Postgres
/// `timestamp with time zone` and `timestamptz`, MySQL `datetime`, Snowflake
/// `TIMESTAMP_NTZ`/`LTZ`/`TZ`, DuckDB `time`). Substring matching also allows
/// harmless schema evolution (such as widening `TIMESTAMP` to `TIMESTAMPTZ`)
/// without falsely invalidating user-taught temporal knowledge like default time.
pub fn is_temporal_type(data_type: &str) -> bool {
    let lower = data_type.trim().to_ascii_lowercase();
    lower.contains("time")
        || lower.contains("date")
        || lower.contains("timestamp")
        || lower.contains("datetime")
}

/// Classifies whether a connector's raw data type string represents a numeric
/// value (integers, floating point, decimals, serials, and numbers).
///
/// We classify types by semantic substring matching because database engines
/// use dialect-specific naming and parameterized widths (e.g. Postgres
/// `bigserial` / `double precision`, MySQL `int(11)` / `decimal(10,2)`,
/// Snowflake `NUMBER(38,0)`, DuckDB `HUGEINT`, SQLite `real`). Facts with
/// numeric requirements (such as measure roles) remain valid when schemas evolve
/// across numeric precision widenings (e.g. `int` to `bigint`).
pub fn is_numeric_type(data_type: &str) -> bool {
    let lower = data_type.trim().to_ascii_lowercase();
    lower.contains("int")
        || lower.contains("float")
        || lower.contains("double")
        || lower.contains("decimal")
        || lower.contains("numeric")
        || lower.contains("real")
        || lower.contains("number")
        || lower.contains("serial")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Column, Table};

    fn make_table(name: &str, cols: &[(&str, &str)]) -> Table {
        Table {
            name: name.to_string(),
            columns: cols
                .iter()
                .map(|(cname, ctype)| Column {
                    name: (*cname).to_string(),
                    data_type: (*ctype).to_string(),
                    nullable: true,
                })
                .collect(),
        }
    }

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

        let modified_table =
            make_table("rental", &[("return_date", "timestamp"), ("notes", "text")]);
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
}
