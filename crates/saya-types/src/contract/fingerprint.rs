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
mod tests {
    use super::*;
    use crate::Column;

    fn make_table(columns: Vec<(&str, &str, bool)>) -> Table {
        Table {
            name: "t".into(),
            columns: columns
                .into_iter()
                .map(|(name, data_type, nullable)| Column {
                    name: name.into(),
                    data_type: data_type.into(),
                    nullable,
                })
                .collect(),
        }
    }

    #[test]
    fn fingerprint_is_stable() {
        let table = make_table(vec![("id", "bigint", false)]);
        let kind = DatabaseObjectKind::Table;
        let fp1 = SchemaFingerprint::of_table(kind, &table);
        let fp2 = SchemaFingerprint::of_table(kind, &table);
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn changes_when_column_renamed() {
        let table_a = make_table(vec![("id", "bigint", false)]);
        let table_b = make_table(vec![("uid", "bigint", false)]);
        let kind = DatabaseObjectKind::Table;
        assert_ne!(
            SchemaFingerprint::of_table(kind, &table_a),
            SchemaFingerprint::of_table(kind, &table_b)
        );
    }

    #[test]
    fn changes_when_column_retyped() {
        let table_a = make_table(vec![("id", "bigint", false)]);
        let table_b = make_table(vec![("id", "int", false)]);
        let kind = DatabaseObjectKind::Table;
        assert_ne!(
            SchemaFingerprint::of_table(kind, &table_a),
            SchemaFingerprint::of_table(kind, &table_b)
        );
    }

    #[test]
    fn changes_when_column_made_nullable() {
        let table_a = make_table(vec![("id", "bigint", false)]);
        let table_b = make_table(vec![("id", "bigint", true)]);
        let kind = DatabaseObjectKind::Table;
        assert_ne!(
            SchemaFingerprint::of_table(kind, &table_a),
            SchemaFingerprint::of_table(kind, &table_b)
        );
    }

    #[test]
    fn changes_when_column_reordered() {
        let table_a = make_table(vec![("a", "int", false), ("b", "text", true)]);
        let table_b = make_table(vec![("b", "text", true), ("a", "int", false)]);
        let kind = DatabaseObjectKind::Table;
        assert_ne!(
            SchemaFingerprint::of_table(kind, &table_a),
            SchemaFingerprint::of_table(kind, &table_b)
        );
    }

    #[test]
    fn changes_when_column_added() {
        let table_a = make_table(vec![("id", "bigint", false)]);
        let table_b = make_table(vec![("id", "bigint", false), ("name", "text", true)]);
        let kind = DatabaseObjectKind::Table;
        assert_ne!(
            SchemaFingerprint::of_table(kind, &table_a),
            SchemaFingerprint::of_table(kind, &table_b)
        );
    }

    #[test]
    fn changes_when_column_removed() {
        let table_a = make_table(vec![("id", "bigint", false), ("name", "text", true)]);
        let table_b = make_table(vec![("id", "bigint", false)]);
        let kind = DatabaseObjectKind::Table;
        assert_ne!(
            SchemaFingerprint::of_table(kind, &table_a),
            SchemaFingerprint::of_table(kind, &table_b)
        );
    }

    #[test]
    fn differs_between_table_and_view() {
        let table = make_table(vec![("id", "bigint", false)]);
        assert_ne!(
            SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table),
            SchemaFingerprint::of_table(DatabaseObjectKind::View, &table)
        );
    }

    #[test]
    fn length_prefix_collision() {
        let table_a = make_table(vec![("a", "bc", false)]);
        let table_b = make_table(vec![("ab", "c", false)]);
        assert_ne!(
            SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table_a),
            SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table_b)
        );
    }

    #[test]
    fn version_is_correct() {
        let table = make_table(vec![("id", "bigint", false)]);
        let fp = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table);
        assert_eq!(fp.version(), FINGERPRINT_VERSION);
    }

    #[test]
    fn display_and_as_str() {
        let table = make_table(vec![("id", "bigint", false)]);
        let fp = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table);
        assert_eq!(fp.as_str(), &format!("{fp}"));
    }
}
