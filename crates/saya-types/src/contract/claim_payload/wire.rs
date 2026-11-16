//! The wire form of a claim payload and the validating deserializer that
//! admits it: persisted or imported payloads re-run the validating
//! constructors instead of bypassing them, and only the canonical blanked
//! tombstone shape may carry empty free text.

use serde::{Deserialize, Deserializer};

use super::ClaimPayload;
use crate::contract::claim_enums::{Cardinality, ColumnRole};
use crate::contract::error::ContractError;
use crate::contract::identity::DatabaseObjectRef;

/// Wire form used only while validating persisted or imported payloads. The
/// public payload cannot derive `Deserialize`: serde would otherwise bypass
/// validating constructors and admit oversized names, control characters, or
/// mismatched column lists. Empty free-text fields are accepted only in the
/// canonical shape emitted by `ClaimPayload::blanked` for tombstones.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RawClaimPayload {
    TableDescription {
        text: String,
    },
    TableAlias {
        alias: String,
    },
    TableGrain {
        description: String,
        #[serde(default)]
        reason: Option<String>,
    },
    ColumnDescription {
        column: String,
        text: String,
    },
    ColumnRole {
        column: String,
        role: ColumnRole,
        #[serde(default)]
        reason: Option<String>,
    },
    DefaultTimeColumn {
        column: String,
        #[serde(default)]
        reason: Option<String>,
    },
    Relationship {
        target: DatabaseObjectRef,
        local_columns: Vec<String>,
        target_columns: Vec<String>,
        cardinality: Cardinality,
    },
    JoinRule {
        target: String,
        local_columns: Vec<String>,
        target_columns: Vec<String>,
        condition: String,
        #[serde(default)]
        reason: Option<String>,
    },
    MetricDefinition {
        name: String,
        definition: String,
        columns: Vec<String>,
        #[serde(default)]
        reason: Option<String>,
    },
    TableUserNote {
        text: String,
    },
}

impl<'de> Deserialize<'de> for ClaimPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawClaimPayload::deserialize(deserializer)?;
        let invalid = |error: ContractError| serde::de::Error::custom(error.to_string());
        match raw {
            RawClaimPayload::TableDescription { text } if text.is_empty() => {
                Ok(Self::TableDescription { text })
            }
            RawClaimPayload::TableDescription { text } => {
                Self::table_description(text).map_err(invalid)
            }
            RawClaimPayload::TableAlias { alias } if alias.is_empty() => {
                Ok(Self::TableAlias { alias })
            }
            RawClaimPayload::TableAlias { alias } => Self::table_alias(alias).map_err(invalid),
            RawClaimPayload::TableGrain {
                description,
                reason,
            } if description.is_empty() && reason.is_none() => Ok(Self::TableGrain {
                description,
                reason,
            }),
            RawClaimPayload::TableGrain {
                description,
                reason,
            } => Self::table_grain(description, reason.as_deref()).map_err(invalid),
            RawClaimPayload::ColumnDescription { column, text } if text.is_empty() => {
                // Validate the structural column with a non-empty sentinel;
                // empty text is reserved for a blanked tombstone.
                Self::column_description(column.clone(), "_").map_err(invalid)?;
                Ok(Self::ColumnDescription { column, text })
            }
            RawClaimPayload::ColumnDescription { column, text } => {
                Self::column_description(column, text).map_err(invalid)
            }
            RawClaimPayload::ColumnRole {
                column,
                role,
                reason,
            } => Self::column_role(column, role, reason.as_deref()).map_err(invalid),
            RawClaimPayload::DefaultTimeColumn { column, reason } => {
                Self::default_time_column(column, reason.as_deref()).map_err(invalid)
            }
            RawClaimPayload::Relationship {
                target,
                local_columns,
                target_columns,
                cardinality,
            } => Self::relationship(target, local_columns, target_columns, cardinality)
                .map_err(invalid),
            RawClaimPayload::JoinRule {
                target,
                local_columns,
                target_columns,
                condition,
                reason,
            } if condition.is_empty() && reason.is_none() => {
                Self::join_rule(
                    target.clone(),
                    local_columns.clone(),
                    target_columns.clone(),
                    "_",
                    None,
                )
                .map_err(invalid)?;
                Ok(Self::JoinRule {
                    target,
                    local_columns,
                    target_columns,
                    condition,
                    reason: None,
                })
            }
            RawClaimPayload::JoinRule {
                target,
                local_columns,
                target_columns,
                condition,
                reason,
            } => Self::join_rule(
                target,
                local_columns,
                target_columns,
                condition,
                reason.as_deref(),
            )
            .map_err(invalid),
            RawClaimPayload::MetricDefinition {
                name,
                definition,
                columns,
                reason,
            } if definition.is_empty() && reason.is_none() => {
                Self::metric_definition(name.clone(), "_", columns.clone(), None)
                    .map_err(invalid)?;
                Ok(Self::MetricDefinition {
                    name,
                    definition,
                    columns,
                    reason: None,
                })
            }
            RawClaimPayload::MetricDefinition {
                name,
                definition,
                columns,
                reason,
            } => Self::metric_definition(name, definition, columns, reason.as_deref())
                .map_err(invalid),
            RawClaimPayload::TableUserNote { text } if text.is_empty() => {
                Ok(Self::TableUserNote { text })
            }
            RawClaimPayload::TableUserNote { text } => Self::table_user_note(text).map_err(invalid),
        }
    }
}
