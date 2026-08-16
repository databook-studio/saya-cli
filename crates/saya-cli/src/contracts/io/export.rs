//! Export orchestration: stored confirmed claims → discovered-shape files
//! (slice 6b).
//!
//! [`export_contracts`] lists every object bound to `identity`, gathers its
//! confirmed claims, renders them to the discovered-shape TOML (see
//! [`super::render`]), and writes one file per object atomically (see
//! [`super::write`]). Confirmed claims only — a candidate is an unreviewed
//! guess and exporting it would launder it into a file someone else will trust.
//!
//! # Filename is derived, not copied (P1: export escape)
//!
//! The export filename is built from the encoded object identity
//! ([`object_id`], `o-<64 hex>`), never from the raw catalog/schema/object
//! name. A database identifier is attacker-influenceable and `validate_name`
//! permits `/`, `\`, `..` and absolute paths, so `destination.join(name)`
//! with a raw name can discard the destination and land outside it. The
//! encoded identity is filesystem-safe by construction — no separator, no
//! `..`, no absolute prefix — so the join cannot escape. The object's
//! qualified name still appears **inside** the file (`object = "..."`),
//! where it is data, not a path. Containment is still verified at write time
//! (see [`super::write`]) as defense in depth and to catch a symlinked
//! destination honestly.

use super::render::ExportObject;
use crate::contracts::ContractOpError;
use saya_store::{ContractStore, SqliteStateStore, object_id};
use saya_types::{ClaimStatus, DatabaseObjectRef, ProfileIdentity};

/// Export every confirmed claim bound to `identity` to one discovered-shape file
/// per object under `destination`. `overwrite` must be `true` to replace an
/// existing file. Returns the paths written and the count of `Relationship`
/// claims skipped (they have no v1 discovered-shape representation).
pub(crate) async fn export_contracts(
    store: &SqliteStateStore,
    identity: &ProfileIdentity,
    destination: &std::path::Path,
    overwrite: bool,
) -> Result<ExportOutcome, ContractOpError> {
    let mut written = Vec::new();
    let mut skipped_relationship = 0usize;
    for object in store.list_objects(identity).await? {
        let confirmed = store
            .list_claims(&object.object, &[ClaimStatus::Confirmed])
            .await?;
        if confirmed.is_empty() {
            continue;
        }
        let export_object = ExportObject {
            object: object.object.clone(),
            claims: confirmed,
        };
        skipped_relationship += export_object
            .claims
            .iter()
            .filter(|c| is_relationship(c))
            .count();
        let Some(body) = super::render::render(&export_object) else {
            // All claims were Relationship (or tombstones) — nothing to write.
            continue;
        };
        // The filename is the encoded identity, not the raw qualified name —
        // see the module docs. The write side canonicalises `destination` and
        // verifies the target's resolved parent stays inside it.
        let path = destination.join(filename_for(&object.object));
        super::write::write_atomic(&path, &body, overwrite)?;
        written.push(path);
    }
    Ok(ExportOutcome {
        written,
        skipped_relationship,
    })
}

/// The result of an export pass: paths written, and a count of claims the v1
/// discovered shape cannot represent (currently `Relationship`) that were
/// skipped rather than silently dropped.
#[derive(Debug, Clone, Default)]
pub(crate) struct ExportOutcome {
    pub written: Vec<std::path::PathBuf>,
    pub skipped_relationship: usize,
}

/// `o-<64 hex>.toml` — the encoded object identity, filesystem-safe by
/// construction. Deriving the name (rather than copying the raw qualified name)
/// is the load-bearing fix for the export-escape P1: a hostile catalog or
/// object name containing `/` or `..` cannot reach the path at all.
fn filename_for(object: &DatabaseObjectRef) -> String {
    format!("{}.toml", object_id(object))
}

fn is_relationship(claim: &saya_store::StoredClaim) -> bool {
    claim
        .payload
        .as_ref()
        .map(|p| p.kind() == "relationship")
        .unwrap_or(false)
}
