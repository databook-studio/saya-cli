use crate::StoreError;
use crate::contracts::records::{ContractObjectId, DeduplicationKey};
use saya_types::{ClaimId, ClaimPayload, DatabaseObjectRef};
use sha2::{Digest, Sha256};

fn field(hash: &mut Sha256, value: &str) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value.as_bytes());
}

fn hex_of(hash: &mut Sha256, prefix: &str) -> String {
    let digest = hash.finalize_reset();
    let mut value = String::from(prefix);
    for byte in digest {
        value.push_str(&format!("{byte:02x}"));
    }
    value
}

pub fn object_id(object: &DatabaseObjectRef) -> ContractObjectId {
    let mut hash = Sha256::new();
    field(&mut hash, object.profile().as_str());
    field(&mut hash, object.catalog());
    field(&mut hash, object.schema());
    field(&mut hash, object.object());
    field(&mut hash, object.kind().as_str());
    ContractObjectId::from_inner(hex_of(&mut hash, "o-"))
}

/// `serialized` is the payload's JSON, used only as the identity of a variant this
/// build does not know about. See the wildcard arm.
pub fn deduplication_key(payload: &ClaimPayload, serialized: &str) -> DeduplicationKey {
    let mut hash = Sha256::new();
    field(&mut hash, payload.kind());
    match payload {
        ClaimPayload::TableDescription { text, .. } => {
            field(&mut hash, &text.trim().to_lowercase());
        }
        ClaimPayload::TableAlias { alias, .. } => {
            field(&mut hash, &alias.trim().to_lowercase());
        }
        ClaimPayload::TableGrain { description, .. } => {
            field(&mut hash, &description.trim().to_lowercase());
        }
        ClaimPayload::ColumnDescription { column, text, .. } => {
            field(&mut hash, &column.trim().to_lowercase());
            field(&mut hash, &text.trim().to_lowercase());
        }
        // ColumnRole hashes the column only — a column has exactly one role, so a
        // second claim with a different role is a contradiction to surface, not a
        // new claim to store. Falling out of UNIQUE(object_id, deduplication_key)
        // is the intended behavior.
        ClaimPayload::ColumnRole { column, .. } => {
            field(&mut hash, &column.trim().to_lowercase());
        }
        // A table has one default time column, so the column alone is the identity;
        // a differing value is a contradiction, handled the same way as ColumnRole.
        ClaimPayload::DefaultTimeColumn { column, .. } => {
            field(&mut hash, &column.trim().to_lowercase());
        }
        ClaimPayload::Relationship {
            target,
            local_columns,
            ..
        } => {
            field(&mut hash, &target.qualified_name().trim().to_lowercase());
            let mut sorted: Vec<String> = local_columns
                .iter()
                .map(|column| column.trim().to_lowercase())
                .collect();
            sorted.sort_unstable();
            for column in &sorted {
                field(&mut hash, column);
            }
        }
        // ClaimPayload is #[non_exhaustive], so this arm is unavoidable. It hashes
        // the whole serialized payload rather than falling through on the kind
        // alone: an unknown variant then fails *open*, storing claims that might
        // have deduplicated, instead of failing closed and silently merging every
        // distinct claim of that kind into the first one ever stored. Add the
        // variant's real identity fields above when saya-types grows one.
        _ => field(&mut hash, serialized),
    }
    DeduplicationKey::from_inner(hex_of(&mut hash, "d-"))
}

pub fn claim_id(object: &ContractObjectId, key: &DeduplicationKey) -> Result<ClaimId, StoreError> {
    let mut hash = Sha256::new();
    field(&mut hash, object.as_str());
    field(&mut hash, key.as_str());
    ClaimId::parse(&hex_of(&mut hash, "c-")).map_err(|_| StoreError::Invalid)
}
