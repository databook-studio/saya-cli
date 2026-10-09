use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Table;
use crate::contract::error::ContractError;
use crate::contract::identity::DatabaseObjectKind;

pub const FINGERPRINT_VERSION: u32 = 1;

const DIGEST_HEX_LEN: usize = 64;

/// A versioned schema digest. The version travels *with* the digest rather than
/// being read from the current constant: a fingerprint loaded from storage was
/// computed under whatever format was current when it was written, and drift
/// detection is only sound if a v1 digest keeps reporting v1.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SchemaFingerprint {
    version: u32,
    digest: String,
}

impl SchemaFingerprint {
    /// Covers only kind, column name, type, and nullability because `SchemaTree`
    /// carries nothing else today.  Adding keys or comments later must bump
    /// `FINGERPRINT_VERSION`.
    pub fn of_table(kind: DatabaseObjectKind, table: &Table) -> Self {
        let mut hash = Sha256::new();
        field_str(&mut hash, &FINGERPRINT_VERSION.to_string());
        field_str(&mut hash, kind.as_str());
        field_str(&mut hash, &table.columns.len().to_string());
        for col in &table.columns {
            field_str(&mut hash, &col.name);
            field_str(&mut hash, &col.data_type);
            field_str(&mut hash, if col.nullable { "1" } else { "0" });
        }
        let digest = hash.finalize();
        let mut hex = String::with_capacity(DIGEST_HEX_LEN);
        for byte in digest {
            hex.push_str(&format!("{byte:02x}"));
        }
        Self {
            version: FINGERPRINT_VERSION,
            digest: hex,
        }
    }

    /// Rebuilds a fingerprint from its two stored columns. Persistence layers hold
    /// the digest and its version separately, so this is how a stored fingerprint
    /// comes back without laundering it through the current format version.
    pub fn from_parts(version: u32, digest: &str) -> Result<Self, ContractError> {
        if digest.len() != DIGEST_HEX_LEN
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(ContractError::InvalidFingerprint);
        }
        Ok(Self {
            version,
            digest: digest.to_owned(),
        })
    }

    /// The format version this digest was computed under — not necessarily the
    /// current `FINGERPRINT_VERSION`.
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// True when this digest was produced by the format the running binary uses.
    /// A mismatch means the digest cannot be compared, only recomputed.
    pub const fn is_current_format(&self) -> bool {
        self.version == FINGERPRINT_VERSION
    }

    pub fn as_str(&self) -> &str {
        &self.digest
    }
}

impl std::fmt::Display for SchemaFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

fn field_str(hash: &mut Sha256, value: &str) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value.as_bytes());
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod property_tests;
