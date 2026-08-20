use serde::{Deserialize, Serialize};

use crate::contract::error::ContractError;

pub const MAX_TEXT_CHARS: usize = 1024;
pub const MAX_REFERENCED_COLUMNS: usize = 32;

/// Serialization version for `ClaimPayload`. Bump when a variant's stored shape changes.
///
/// Version 2 (Phase 5a) does not change a payload's own shape; it is bumped
/// because `referenced_columns_json` changed from a bare name list
/// (`["a","b"]`) to an array of typed snapshots (`[{"name","data_type",
/// "nullable"}]`). A claim's `payload_version` records which shape the row
/// was written under, so a future reader knows whether to expect snapshots
/// or names. Old version-1 rows still decode: the store upgrades a bare
/// name to a name-only snapshot flagged unknown.
///
/// Version 3 (claim-reasons) adds an optional `reason` to the directive
/// variants (`TableGrain`, `ColumnRole`, `DefaultTimeColumn`). The field is
/// `#[serde(default)]`, so a row written under version 2 decodes with
/// `reason: None` — the state of every directive claim made before the field
/// existed. No migration is owed: the read path tolerates the missing field.
pub const CLAIM_PAYLOAD_VERSION: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct ClaimId(String);

impl ClaimId {
    pub fn parse(value: &str) -> Result<Self, ContractError> {
        if value.is_empty() || value.len() > 128 {
            return Err(ContractError::InvalidClaimId);
        }
        if !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(ContractError::InvalidClaimId);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ClaimId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for ClaimId {
    type Error = ContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

/// A claim is one bounded statement, and control characters are how a payload
/// smuggles structure into a rendered context block.
pub(crate) fn validate_text(value: &str) -> Result<(), ContractError> {
    if value.is_empty() {
        return Err(ContractError::EmptyText);
    }
    if value.chars().count() > MAX_TEXT_CHARS {
        return Err(ContractError::TextTooLong);
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(ContractError::ControlCharacter);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_id_parse_valid() {
        let id = ClaimId::parse("c-abc123").unwrap();
        assert_eq!(id.as_str(), "c-abc123");
    }

    #[test]
    fn claim_id_rejects_empty() {
        assert!(ClaimId::parse("").is_err());
    }

    #[test]
    fn claim_id_rejects_too_long() {
        let long = "a".repeat(129);
        assert!(ClaimId::parse(&long).is_err());
    }

    #[test]
    fn claim_id_rejects_invalid_chars() {
        assert!(ClaimId::parse("hello!").is_err());
        assert!(ClaimId::parse("a b").is_err());
    }

    #[test]
    fn claim_id_serde_round_trip() {
        let id = ClaimId::parse("c-abc123").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: ClaimId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn claim_id_serde_rejects_invalid() {
        let result: Result<ClaimId, _> = serde_json::from_str(r#""invalid!id""#);
        assert!(result.is_err());
    }
}
