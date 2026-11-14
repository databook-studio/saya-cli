//! Conversions between `ClaimPayload` and its portable form: identity is
//! stripped on the way out and rebuilt through the validating constructors on
//! the way in, so a resolved claim is exactly as valid as any other claim.

use super::payload::{PortablePayload, is_target_bearing};
use super::{ContextError, PortableObject};
use crate::DatabaseObjectKind;
use crate::contract::claim_payload::ClaimPayload;
use crate::contract::error::ContractError;
use crate::contract::identity::DatabaseObjectRef;

impl PortablePayload {
    /// Builds the portable form of any claim: target-bearing payloads are
    /// stripped of their identity — a `DatabaseObjectRef` becomes a logical
    /// [`PortableObject`], and a join rule's qualified-name target is split
    /// when it has exactly three non-empty parts — and every other kind rides
    /// unchanged in `Claim`. This is the only way another crate builds a
    /// portable payload.
    pub fn from_claim(claim: &ClaimPayload) -> Result<Self, ContextError> {
        Ok(match claim {
            ClaimPayload::Relationship {
                target,
                local_columns,
                target_columns,
                cardinality,
            } => Self::Relationship {
                target: PortableObject {
                    catalog: Some(target.catalog().to_string()),
                    schema: Some(target.schema().to_string()),
                    name: target.object().to_string(),
                    kind: target.kind(),
                },
                local_columns: local_columns.clone(),
                target_columns: target_columns.clone(),
                cardinality: *cardinality,
            },
            ClaimPayload::JoinRule {
                target,
                local_columns,
                target_columns,
                condition,
                reason,
            } => Self::JoinRule {
                target: portable_join_target(target),
                local_columns: local_columns.clone(),
                target_columns: target_columns.clone(),
                condition: condition.clone(),
                reason: reason.clone(),
            },
            other => Self::Claim {
                claim: other.clone(),
            },
        })
    }

    /// Rebuilds the claim, resolving each portable target through `resolve`:
    /// a `Relationship` takes the resolved ref directly, a `JoinRule` takes
    /// the resolved ref's qualified name. Both rebuild through the existing
    /// validating constructors, so an unresolved target or a payload the
    /// constructors refuse is a typed error — never a mutated claim.
    pub fn into_claim(
        self,
        resolve: impl Fn(&PortableObject) -> Option<DatabaseObjectRef>,
    ) -> Result<ClaimPayload, ContextError> {
        match self {
            Self::Claim { claim } if is_target_bearing(&claim) => {
                Err(ContextError::PayloadNotPortable)
            }
            Self::Claim { claim } => Ok(claim),
            Self::Relationship {
                target,
                local_columns,
                target_columns,
                cardinality,
            } => {
                let resolved = resolve(&target).ok_or(ContextError::UnresolvedTarget)?;
                ClaimPayload::relationship(resolved, local_columns, target_columns, cardinality)
                    .map_err(payload_error)
            }
            Self::JoinRule {
                target,
                local_columns,
                target_columns,
                condition,
                reason,
            } => {
                let resolved = resolve(&target).ok_or(ContextError::UnresolvedTarget)?;
                ClaimPayload::join_rule(
                    resolved.qualified_name(),
                    local_columns,
                    target_columns,
                    condition,
                    reason.as_deref(),
                )
                .map_err(payload_error)
            }
        }
    }
}

/// Splits a join rule's qualified-name target into its logical parts when it
/// has exactly three non-empty parts; anything else travels as one bare name
/// the importing profile's resolver interprets. The kind is informational for
/// join rules — the resolved ref decides — so `Table` is a placeholder, never
/// a claim about the target.
fn portable_join_target(target: &str) -> PortableObject {
    let parts: Vec<&str> = target.split('.').collect();
    let (catalog, schema, name) = match parts.as_slice() {
        [catalog, schema, name]
            if !catalog.is_empty() && !schema.is_empty() && !name.is_empty() =>
        {
            (
                Some((*catalog).to_string()),
                Some((*schema).to_string()),
                (*name).to_string(),
            )
        }
        _ => (None, None, target.to_string()),
    };
    PortableObject {
        catalog,
        schema,
        name,
        kind: DatabaseObjectKind::Table,
    }
}

/// Maps a validating constructor's refusal onto this document's errors; the
/// constructors are the same ones every claim passes, so their bounds are
/// this document's bounds.
fn payload_error(error: ContractError) -> ContextError {
    match error {
        ContractError::ControlCharacter => ContextError::ControlCharacter,
        ContractError::EmptyName | ContractError::NameTooLong => ContextError::InvalidObjectName,
        ContractError::EmptyColumns
        | ContractError::ColumnCountMismatch
        | ContractError::TooManyColumns => ContextError::InvalidColumns,
        ContractError::EmptyText | ContractError::TextTooLong => ContextError::InvalidText,
        _ => ContextError::InvalidPayload,
    }
}
