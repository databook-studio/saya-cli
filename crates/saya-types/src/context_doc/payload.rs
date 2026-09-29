//! The portable payload: one claim's content as it travels between profiles.
//!
//! Most claims ride unchanged inside `Claim` — `ClaimPayload`'s validated
//! serde already bounds and control-char-checks them. The two claims that
//! name a *second* object (`Relationship`, `JoinRule`) get their own
//! variants: their target is a logical [`PortableObject`] the importing
//! profile resolves against its own schema, so no machine-bound profile
//! identity and no source-profile qualified name ever leave the file.

use serde::{Deserialize, Serialize};

use super::{ContextError, PortableObject};
use crate::contract::claim_enums::Cardinality;
use crate::contract::claim_payload::ClaimPayload;
use crate::contract::error::ContractError;
use crate::contract::identity::validate_name;
use crate::{MAX_REFERENCED_COLUMNS, MAX_TEXT_CHARS};

/// A claim payload in portable form: `Claim` wraps the payload kinds that
/// reference no second object, and `Relationship`/`JoinRule` carry their
/// target as a logical object. Built with [`PortablePayload::from_claim`],
/// read back with [`PortablePayload::into_claim`]; the variants are
/// `non_exhaustive` so another crate cannot build one around an ungated
/// payload. Deserialization accepts the raw shapes; `validate` is the gate
/// that refuses a target-bearing payload inside `Claim` — identity can
/// parse, but it can never pass or leave.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum PortablePayload {
    /// Any payload kind that references no second object, validated by
    /// `ClaimPayload`'s own serde. A target-bearing payload here is refused.
    #[non_exhaustive]
    Claim { claim: ClaimPayload },
    /// A relationship claim whose target travels as a logical object.
    #[non_exhaustive]
    Relationship {
        target: PortableObject,
        local_columns: Vec<String>,
        target_columns: Vec<String>,
        cardinality: Cardinality,
    },
    /// A join rule whose target travels as a logical object; the original
    /// claim's qualified-name target is rebuilt from the resolved ref.
    #[non_exhaustive]
    JoinRule {
        target: PortableObject,
        local_columns: Vec<String>,
        target_columns: Vec<String>,
        condition: String,
        #[serde(default)]
        reason: Option<String>,
    },
}

/// Whether a payload names a second object and must therefore travel in its
/// own portable variant: `Relationship` embeds a profile identity outright,
/// and `JoinRule`'s target is a qualified name bound to the source profile's
/// naming — neither may ride inside `Claim`.
pub(super) fn is_target_bearing(claim: &ClaimPayload) -> bool {
    matches!(
        claim,
        ClaimPayload::Relationship { .. } | ClaimPayload::JoinRule { .. }
    )
}

impl PortablePayload {
    /// Checks a portable payload's shape against the same bounds the claim
    /// constructors enforce — column pairing, name bounds, text bounds — so a
    /// parsed document cannot carry a payload `into_claim` would refuse. A
    /// `Claim`-wrapped payload is trusted: `ClaimPayload`'s validated serde
    /// already bounds it.
    pub(super) fn validate(&self) -> Result<(), ContextError> {
        match self {
            Self::Claim { claim } if is_target_bearing(claim) => {
                Err(ContextError::PayloadNotPortable)
            }
            Self::Claim { .. } => Ok(()),
            Self::Relationship {
                target,
                local_columns,
                target_columns,
                ..
            } => {
                target.validate()?;
                validate_column_pair(local_columns, target_columns, true)
            }
            Self::JoinRule {
                target,
                local_columns,
                target_columns,
                condition,
                reason,
            } => {
                target.validate()?;
                if !(local_columns.is_empty() && target_columns.is_empty()) {
                    validate_column_pair(local_columns, target_columns, false)?;
                }
                validate_text_field(condition)?;
                if let Some(reason) = reason {
                    validate_text_field(reason)?;
                }
                Ok(())
            }
        }
    }
}

/// Checks one column pair: relationship columns must be non-empty, a join
/// rule's may both be empty, and either way the lists pair positionally and
/// hold at most [`MAX_REFERENCED_COLUMNS`] valid names.
fn validate_column_pair(
    local_columns: &[String],
    target_columns: &[String],
    require_non_empty: bool,
) -> Result<(), ContextError> {
    if require_non_empty && (local_columns.is_empty() || target_columns.is_empty()) {
        return Err(ContextError::InvalidColumns);
    }
    if local_columns.len() != target_columns.len() || local_columns.len() > MAX_REFERENCED_COLUMNS {
        return Err(ContextError::InvalidColumns);
    }
    for name in local_columns.iter().chain(target_columns) {
        match validate_name(name) {
            Ok(()) => {}
            Err(ContractError::ControlCharacter) => return Err(ContextError::ControlCharacter),
            Err(_) => return Err(ContextError::InvalidColumns),
        }
    }
    Ok(())
}

/// Checks one free-text payload field the way `validate_text` does: non-empty,
/// bounded, and free of control characters. The blanked tombstone shape is a
/// store concern and has no place in an exchange document, so an empty
/// condition is refused here.
fn validate_text_field(value: &str) -> Result<(), ContextError> {
    if value.is_empty() || value.chars().count() > MAX_TEXT_CHARS {
        return Err(ContextError::InvalidText);
    }
    if value.chars().any(char::is_control) {
        return Err(ContextError::ControlCharacter);
    }
    Ok(())
}
