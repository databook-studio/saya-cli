//! The `contracts import` operation (B2c invariants 2–4): validate the whole
//! document first, map its items onto one profile's schema, and apply what
//! resolved as one transactional pending batch.
//!
//! Order is the invariant: the bounded read and the document's own validation
//! complete before any store access, so an invalid file writes nothing at all.
//! The mapping (see `map`) resolves every logical object against the profile's
//! schema tree and reports what does not resolve; the batch lands what
//! resolved as Pending with the team-file origin — never Active, whatever the
//! file claimed — and reports skips and conflicts instead of overwriting.
//!
//! `--preview` is read-only: it classifies every mapped item the way the batch
//! would (same id derivation, same state-and-value comparison the batch's
//! `apply_one` makes) without opening a transaction. The store's batch remains
//! the authority when the import applies; the pre-check exists so the preview
//! report shows honest skip/conflict counts, not zeroes.

use std::{fs::File, io::Read as _, path::Path};

use saya_store::{BatchItemOutcome, KnowledgeItemStore, NewKnowledgeItem, SqliteStateStore};
use saya_types::{
    ContextDocumentV1, ContextItem, KnowledgeState, MAX_DOCUMENT_BYTES, ProfileIdentity, SchemaTree,
};

use super::{PortableError, map::PlannedItem, map_items};

/// One import's full report, aligned with what the file carried.
pub(crate) struct ImportOutcome {
    /// `(resolved object, filed slot)` pairs, in document order.
    pub inserted: Vec<(String, String)>,
    /// `(object, slot)` pairs already on file identically.
    pub skipped: Vec<(String, String)>,
    /// `(object, slot, existing claim id)` — the slot a local claim already
    /// holds with a different value, or holds forgotten.
    pub conflicts: Vec<(String, String, String)>,
    /// Items that could not map, with the reason.
    pub unavailable: Vec<super::Unavailability>,
}

impl ImportOutcome {
    /// How many items landed (written or already present).
    pub(crate) fn landed(&self) -> usize {
        self.inserted.len() + self.skipped.len()
    }

    /// How many items the input carried in total.
    pub(crate) fn total(&self) -> usize {
        self.landed() + self.conflicts.len() + self.unavailable.len()
    }
}

/// Reads and validates a context document from `path`: the byte cap before a
/// byte parses (an oversize read is refused, never truncated), then the
/// document's own validation. No store access happens here.
pub(crate) fn read_context(path: &Path) -> Result<ContextDocumentV1, PortableError> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ContextDocumentV1::from_json_bytes(&bytes).map_err(PortableError::from)
}

/// Maps `items` against `tree` and — unless `preview` — applies the mapped
/// batch to `store`. The batch is one transaction: any storage failure rolls
/// everything back and surfaces as a [`PortableError`], so an applied import
/// is all-or-nothing.
pub(crate) async fn import_document(
    store: &SqliteStateStore,
    identity: &ProfileIdentity,
    items: &[ContextItem],
    tree: &SchemaTree,
    preview: bool,
) -> Result<ImportOutcome, PortableError> {
    let mapped = map_items(items, tree, identity);
    let mut outcome = ImportOutcome {
        inserted: Vec::new(),
        skipped: Vec::new(),
        conflicts: Vec::new(),
        unavailable: mapped.unavailable,
    };
    if mapped.planned.is_empty() {
        return Ok(outcome);
    }
    let results = if preview {
        classify_read_only(store, &mapped.planned).await?
    } else {
        let batch: Vec<NewKnowledgeItem> = mapped
            .planned
            .iter()
            .map(|planned| planned.item.clone())
            .collect();
        store.apply_pending_batch(&batch).await?.items
    };
    for (planned, result) in mapped.planned.iter().zip(results.iter()) {
        let slot = planned.slot.clone();
        match result {
            BatchItemOutcome::Inserted { .. } => {
                outcome.inserted.push((planned.label.clone(), slot))
            }
            BatchItemOutcome::Skipped => outcome.skipped.push((planned.label.clone(), slot)),
            BatchItemOutcome::Conflict { existing_id } => {
                outcome
                    .conflicts
                    .push((planned.label.clone(), slot, existing_id.clone()))
            }
        }
    }
    Ok(outcome)
}

/// The preview's read-only classification, mirroring the batch's `apply_one`
/// three-way decision: the id the write would land on (derived the same way
/// the store derives it), the row's state, and the value comparison. Reads
/// only; writes nothing.
async fn classify_read_only(
    store: &SqliteStateStore,
    planned: &[PlannedItem],
) -> Result<Vec<BatchItemOutcome>, PortableError> {
    let mut results = Vec::with_capacity(planned.len());
    for item in planned {
        let serialized = serde_json::to_string(&item.item.value).map_err(|_| {
            PortableError::Store(saya_store::KnowledgeStoreError::Store(
                saya_store::StoreError::invalid(),
            ))
        })?;
        let id = saya_store::knowledge_item_id_for(&item.item.object, &item.item.slot, &serialized);
        let outcome = match store.get_knowledge_item(&id).await? {
            None => BatchItemOutcome::Inserted { id },
            Some(row) if row.state == KnowledgeState::Dismissed => {
                BatchItemOutcome::Conflict { existing_id: id }
            }
            Some(row) if row.value == item.item.value => BatchItemOutcome::Skipped,
            Some(row) => BatchItemOutcome::Conflict {
                existing_id: row.id,
            },
        };
        results.push(outcome);
    }
    Ok(results)
}
