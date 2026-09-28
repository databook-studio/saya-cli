//! Document operations on the investigation repository: create, update,
//! get, and delete. Every write validates through
//! [`InvestigationDefinitionV1::to_json_pretty`] first, then stages private
//! bytes beside the target and publishes atomically; every read is bounded
//! and reports — never repairs — a document it cannot parse. Every mutation
//! runs entirely under the repository lock, so read/check/publish is one
//! critical section across processes.

use saya_types::investigation::{InvestigationDefinitionV1, InvestigationId, MAX_DEFINITION_BYTES};

use crate::replace::{AtomicReplace, Replacer};
use crate::{StoreError, private_file};

use super::{InvestigationRepository, MAX_DOCUMENTS, store_error_of};

impl InvestigationRepository {
    /// Saves a new document under the repository lock. Refuses an existing
    /// id (`Conflict`) — including one that appears after the checks, since
    /// the exclusive publish never replaces — a full collection
    /// (`LimitExceeded`), and a definition that fails its own serialization
    /// validation (`Invalid`, or `LimitExceeded` when the serialized
    /// document exceeds the byte cap).
    pub fn create(&self, definition: &InvestigationDefinitionV1) -> Result<(), StoreError> {
        let _lock = self.lock_for_mutation()?;
        self.create_locked(definition)
    }

    /// The create critical section; the caller holds the repository lock.
    fn create_locked(&self, definition: &InvestigationDefinitionV1) -> Result<(), StoreError> {
        let path = self.document_path(definition.id.as_str())?;
        if std::fs::symlink_metadata(&path).is_ok() {
            return Err(StoreError::conflict());
        }
        if self.scan_candidates()?.len() >= MAX_DOCUMENTS {
            return Err(StoreError::limit_exceeded());
        }
        let data = definition.to_json_pretty().map_err(store_error_of)?;
        private_file::stage_and_publish_no_replace(&path, data.as_bytes())
    }

    /// Replaces a document with its next revision, under the repository
    /// lock. Refuses a missing document (`NotFound`), a stale or
    /// non-advancing revision or a document whose internal id disagrees
    /// with its filename (`Conflict`), and a definition that fails
    /// validation (`Invalid`). On any error the original file is
    /// byte-for-byte unchanged.
    pub fn update(
        &self,
        definition: &InvestigationDefinitionV1,
        expected_revision: u32,
    ) -> Result<(), StoreError> {
        let _lock = self.lock_for_mutation()?;
        self.update_inner(definition, expected_revision, &AtomicReplace)
    }

    pub(crate) fn update_inner(
        &self,
        definition: &InvestigationDefinitionV1,
        expected_revision: u32,
        replacer: &dyn Replacer,
    ) -> Result<(), StoreError> {
        let current = self
            .read_document(&definition.id)?
            .ok_or_else(StoreError::not_found)?;
        let next = match expected_revision.checked_add(1) {
            Some(next) => next,
            None => return Err(StoreError::conflict()),
        };
        if current.revision != expected_revision
            || definition.revision != next
            || definition.id != current.id
        {
            return Err(StoreError::conflict());
        }
        let path = self.document_path(definition.id.as_str())?;
        let data = definition.to_json_pretty().map_err(store_error_of)?;
        private_file::stage_and_publish(&path, data.as_bytes(), replacer)
    }

    #[cfg(test)]
    pub(crate) fn update_with_replacer(
        &self,
        definition: &InvestigationDefinitionV1,
        expected_revision: u32,
        replacer: &dyn Replacer,
    ) -> Result<(), StoreError> {
        self.update_inner(definition, expected_revision, replacer)
    }

    /// Reads one document. `NotFound` when absent; a file over the byte cap
    /// is `LimitExceeded`; an unknown version is `VersionUnsupported`; a
    /// corrupt or internally inconsistent document is `Invalid` — reported,
    /// never quarantined or rewritten.
    pub fn get(&self, id: &InvestigationId) -> Result<InvestigationDefinitionV1, StoreError> {
        let definition = self.read_document(id)?.ok_or_else(StoreError::not_found)?;
        if definition.id != *id {
            return Err(StoreError::invalid());
        }
        Ok(definition)
    }

    /// Deletes one document under the repository lock — the revision check,
    /// the removal, and the binding clear are one critical section — after
    /// checking its revision. A document whose revision cannot be verified
    /// (corrupt, unknown version, oversize) is refused, never deleted.
    pub fn delete(&self, id: &InvestigationId, expected_revision: u32) -> Result<(), StoreError> {
        let _lock = self.lock_for_mutation()?;
        let current = self.read_document(id)?.ok_or_else(StoreError::not_found)?;
        if current.id != *id || current.revision != expected_revision {
            return Err(StoreError::conflict());
        }
        let path = self.document_path(id.as_str())?;
        std::fs::remove_file(&path).map_err(private_file::io_error)?;
        self.clear_binding_locked(id)
    }

    /// Bounded read plus strict parse of the document at `<id>.json`.
    /// `Ok(None)` when the file is absent. The caller owns the
    /// id-consistency check, because the right refusal differs by
    /// operation (`Conflict` on update, `Invalid` on read paths).
    pub(crate) fn read_document(
        &self,
        id: &InvestigationId,
    ) -> Result<Option<InvestigationDefinitionV1>, StoreError> {
        let path = self.document_path(id.as_str())?;
        let bytes = private_file::bounded_read(&path, MAX_DEFINITION_BYTES)?;
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let definition =
            InvestigationDefinitionV1::from_json_bytes(&bytes).map_err(store_error_of)?;
        Ok(Some(definition))
    }
}
