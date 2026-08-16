use serde::{Deserialize, Serialize};

use crate::contract::claim_enums::ColumnRole;
use crate::contract::claim_payload::ClaimPayload;
use crate::contract::slot::KnowledgeSlot;
use crate::contract::type_classifier::{is_numeric_type, is_temporal_type};
use crate::schema::{SchemaTree, Table};

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
    /// Derives the structural schema binding for a paired `(KnowledgeSlot, ClaimPayload)`.
    ///
    /// Table-level slots (`TableDescription`, `TableAlias`, `TableGrain`) yield
    /// `SchemaBinding::Table`.
    /// Column-level slots yield `SchemaBinding::Column` with appropriate semantic
    /// `ColumnRequirement`:
    /// - `TableDefaultTime` and `ColumnRole::Timestamp` require `ColumnRequirement::Time`.
    /// - `ColumnRole::Measure` requires `ColumnRequirement::Numeric`.
    /// - `ColumnDescription` and other `ColumnRole` variants require `ColumnRequirement::Exists`.
    ///
    /// Returns `None` if the slot and payload types disagree, if column names mismatch,
    /// or if the payload is not slot-bound (e.g. `ClaimPayload::Relationship`).
    pub fn derive(slot: &KnowledgeSlot, payload: &ClaimPayload) -> Option<Self> {
        match (slot, payload) {
            (KnowledgeSlot::TableDescription, ClaimPayload::TableDescription { .. })
            | (KnowledgeSlot::TableAlias, ClaimPayload::TableAlias { .. })
            | (KnowledgeSlot::TableGrain, ClaimPayload::TableGrain { .. }) => Some(Self::Table),
            (KnowledgeSlot::TableDefaultTime, ClaimPayload::DefaultTimeColumn { column }) => {
                Some(Self::Column {
                    column: column.clone(),
                    requirement: ColumnRequirement::Time,
                })
            }
            (
                KnowledgeSlot::ColumnDescription { column: slot_col },
                ClaimPayload::ColumnDescription { column, .. },
            ) if slot_col == column => Some(Self::Column {
                column: column.clone(),
                requirement: ColumnRequirement::Exists,
            }),
            (
                KnowledgeSlot::ColumnRole { column: slot_col },
                ClaimPayload::ColumnRole { column, role },
            ) if slot_col == column => {
                let requirement = match role {
                    ColumnRole::Timestamp => ColumnRequirement::Time,
                    ColumnRole::Measure => ColumnRequirement::Numeric,
                    ColumnRole::Identifier | ColumnRole::Dimension | ColumnRole::Sensitive => {
                        ColumnRequirement::Exists
                    }
                };
                Some(Self::Column {
                    column: column.clone(),
                    requirement,
                })
            }
            _ => None,
        }
    }

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

    /// Validates this binding against a table found inside a [`SchemaTree`].
    ///
    /// Looks up the table at `(catalog, schema, table)` using case-insensitive
    /// resolution via [`SchemaTree::find_table`]. If found, validates against
    /// that live table; if not found, returns [`BindingValidity::Invalid`].
    pub fn validate_in_tree(
        &self,
        tree: &SchemaTree,
        catalog: &str,
        schema: &str,
        table: &str,
    ) -> BindingValidity {
        match tree.find_table(catalog, schema, table) {
            Some(t) => self.validate(t),
            None => BindingValidity::Invalid,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::claim_enums::Cardinality;
    use crate::contract::identity::{DatabaseObjectKind, DatabaseObjectRef, ProfileIdentity};
    use crate::schema::{Column, Database, Schema, Table};
    use proptest::prelude::*;

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

    fn make_target() -> DatabaseObjectRef {
        let profile = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
        DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap()
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

    /// Chunk 2 test: TableDescription, TableAlias, and TableGrain derive SchemaBinding::Table.
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
        let grain_payload = ClaimPayload::table_grain("one row per rental event").unwrap();
        assert_eq!(
            SchemaBinding::derive(&grain_slot, &grain_payload),
            Some(SchemaBinding::Table)
        );
    }

    /// Chunk 2 test: TableDefaultTime derives SchemaBinding::Column with ColumnRequirement::Time.
    #[test]
    fn test_derive_default_time() {
        let slot = KnowledgeSlot::TableDefaultTime;
        let payload = ClaimPayload::default_time_column("return_date").unwrap();
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
        let ts_payload = ClaimPayload::column_role("created_at", ColumnRole::Timestamp).unwrap();
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
        let measure_payload = ClaimPayload::column_role("amount", ColumnRole::Measure).unwrap();
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
            let role_payload = ClaimPayload::column_role(col, role).unwrap();
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
        let default_time_payload = ClaimPayload::default_time_column("created_at").unwrap();
        assert_eq!(
            SchemaBinding::derive(&desc_slot, &default_time_payload),
            None
        );

        let col_desc_slot = KnowledgeSlot::ColumnDescription {
            column: "tier".to_string(),
        };
        let col_role_payload = ClaimPayload::column_role("tier", ColumnRole::Identifier).unwrap();
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
        let role_payload = ClaimPayload::column_role("column_b", ColumnRole::Timestamp).unwrap();
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

    /// Spec Test 6: Derived binding end-to-end validation over live tables.
    #[test]
    fn test_derived_binding_end_to_end_validation() {
        // 1. TableGrain -> derive -> validate
        let grain_slot = KnowledgeSlot::TableGrain;
        let grain_payload = ClaimPayload::table_grain("one row per event").unwrap();
        let grain_binding = SchemaBinding::derive(&grain_slot, &grain_payload).unwrap();
        let table = make_table("events", &[("id", "int")]);
        assert_eq!(grain_binding.validate(&table), BindingValidity::Valid);

        // 2. DefaultTimeColumn -> derive -> validate
        let time_slot = KnowledgeSlot::TableDefaultTime;
        let time_payload = ClaimPayload::default_time_column("event_time").unwrap();
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
        let measure_payload = ClaimPayload::column_role("revenue", ColumnRole::Measure).unwrap();
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

    /// Tree Resolution Test: Tests case-insensitive catalog/schema/table resolution in SchemaTree.
    #[test]
    fn test_validate_in_tree_resolution() {
        let table = make_table("Rental", &[("Return_Date", "timestamp")]);
        let tree = SchemaTree {
            databases: vec![Database {
                name: "MainDb".to_string(),
                schemas: vec![Schema {
                    name: "Public".to_string(),
                    tables: vec![table],
                }],
            }],
        };

        let binding = SchemaBinding::Column {
            column: "return_date".to_string(),
            requirement: ColumnRequirement::Time,
        };

        // Exact match
        assert_eq!(
            binding.validate_in_tree(&tree, "MainDb", "Public", "Rental"),
            BindingValidity::Valid
        );

        // Case-insensitive match on all 3 components
        assert_eq!(
            binding.validate_in_tree(&tree, "maindb", "public", "rental"),
            BindingValidity::Valid
        );

        // Missing catalog
        assert_eq!(
            binding.validate_in_tree(&tree, "other_db", "public", "rental"),
            BindingValidity::Invalid
        );

        // Missing schema
        assert_eq!(
            binding.validate_in_tree(&tree, "maindb", "other_schema", "rental"),
            BindingValidity::Invalid
        );

        // Missing table
        assert_eq!(
            binding.validate_in_tree(&tree, "maindb", "public", "customer"),
            BindingValidity::Invalid
        );
    }

    // Property Test Generators
    fn arb_col_name() -> impl Strategy<Value = String> {
        "[a-z_][a-z0-9_]{0,15}"
    }

    fn arb_data_type() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("timestamp".to_string()),
            Just("timestamptz".to_string()),
            Just("date".to_string()),
            Just("time".to_string()),
            Just("datetime".to_string()),
            Just("int".to_string()),
            Just("bigint".to_string()),
            Just("numeric(10,2)".to_string()),
            Just("float".to_string()),
            Just("text".to_string()),
            Just("varchar(50)".to_string()),
            Just("boolean".to_string()),
            Just("json".to_string()),
        ]
    }

    fn arb_column() -> impl Strategy<Value = Column> {
        (arb_col_name(), arb_data_type(), any::<bool>()).prop_map(|(name, data_type, nullable)| {
            Column {
                name,
                data_type,
                nullable,
            }
        })
    }

    fn arb_table() -> impl Strategy<Value = Table> {
        (arb_col_name(), prop::collection::vec(arb_column(), 0..10))
            .prop_map(|(name, columns)| Table { name, columns })
    }

    fn arb_column_requirement() -> impl Strategy<Value = ColumnRequirement> {
        prop_oneof![
            Just(ColumnRequirement::Exists),
            Just(ColumnRequirement::Time),
            Just(ColumnRequirement::Numeric),
        ]
    }

    fn arb_schema_binding() -> impl Strategy<Value = SchemaBinding> {
        prop_oneof![
            Just(SchemaBinding::Table),
            (arb_col_name(), arb_column_requirement()).prop_map(|(column, requirement)| {
                SchemaBinding::Column {
                    column,
                    requirement,
                }
            })
        ]
    }

    proptest! {
        /// Core Invariant: Appending an unrelated column with a distinct name
        /// NEVER changes the validation verdict across arbitrary generated tables.
        #[test]
        fn prop_unrelated_column_addition_never_alters_validity(
            table in arb_table(),
            binding in arb_schema_binding(),
            new_col_name in arb_col_name(),
            new_col_type in arb_data_type(),
            new_col_nullable in any::<bool>(),
        ) {
            // Ensure the new column name is distinct from what the binding depends upon
            if let SchemaBinding::Column { ref column, .. } = binding {
                prop_assume!(!new_col_name.eq_ignore_ascii_case(column));
            }

            let initial_verdict = binding.validate(&table);

            let mut modified_table = table.clone();
            modified_table.columns.push(Column {
                name: new_col_name,
                data_type: new_col_type,
                nullable: new_col_nullable,
            });

            let after_verdict = binding.validate(&modified_table);
            prop_assert_eq!(initial_verdict, after_verdict);
        }

        /// Dropping the referenced column from any table ALWAYS marks a Column binding Invalid.
        #[test]
        fn prop_dropped_referenced_column_always_invalid(
            table in arb_table(),
            col_name in arb_col_name(),
            requirement in arb_column_requirement(),
        ) {
            let binding = SchemaBinding::Column {
                column: col_name.clone(),
                requirement,
            };

            // Remove all columns that match the target column (case-insensitively)
            let mut stripped_table = table.clone();
            stripped_table.columns.retain(|c| !c.name.eq_ignore_ascii_case(&col_name));

            prop_assert_eq!(binding.validate(&stripped_table), BindingValidity::Invalid);
        }

        /// Arbitrary SchemaBinding round-trips identically through JSON serde.
        #[test]
        fn prop_binding_serde_round_trip(
            binding in arb_schema_binding(),
        ) {
            let serialized = serde_json::to_string(&binding).expect("serialization succeeds");
            let deserialized: SchemaBinding = serde_json::from_str(&serialized).expect("deserialization succeeds");
            prop_assert_eq!(binding, deserialized);
        }
    }
}
