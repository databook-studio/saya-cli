//! The per-machine local binding for a saved investigation (D2): which
//! profile the investigation was last saved/reviewed against and what it
//! was reviewed with. This is machine state beside the portable document —
//! never exported, never included in listings, never readable through the
//! document API, and never carrying authority on another machine.

use saya_types::investigation::InvestigationId;
use serde::{Deserialize, Serialize};

use crate::bounded::BoundedWriter;
use crate::replace::{AtomicReplace, Replacer};
use crate::{StoreError, private_file};

use super::InvestigationRepository;

/// The binding document's serialized-size ceiling.
pub const MAX_BINDING_BYTES: usize = 16 * 1024;

/// Per-machine state beside a saved investigation: the profile the document
/// was reviewed against, an opaque identity for that profile, and the
/// revision + schema fingerprint + time of the review. Denies unknown
/// fields, so a binding written by a newer saya is refused on read, never
/// downgraded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalBinding {
    #[serde(default = "binding_version")]
    pub version: u32,
    pub id: InvestigationId,
    pub profile: String,
    pub profile_identity: String,
    pub reviewed_revision: u32,
    pub reviewed_schema_fingerprint: Option<String>,
    pub reviewed_unix_ms: i64,
}

impl LocalBinding {
    /// The only binding version this store reads or writes; anything else
    /// is refused on read and never written.
    pub const VERSION: u32 = 1;
}

fn binding_version() -> u32 {
    LocalBinding::VERSION
}

impl InvestigationRepository {
    /// Reads the local binding for `id`, if any. A file over the byte cap
    /// is `LimitExceeded`; an unknown version is `VersionUnsupported`; a
    /// corrupt file or one whose internal id disagrees with the requested
    /// id is `Invalid` — reported, never repaired.
    pub fn get_binding(&self, id: &InvestigationId) -> Result<Option<LocalBinding>, StoreError> {
        let path = self.binding_path(id);
        let bytes = private_file::bounded_read(&path, MAX_BINDING_BYTES)?;
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let binding: LocalBinding =
            serde_json::from_slice(&bytes).map_err(|_| StoreError::invalid())?;
        if binding.version != LocalBinding::VERSION {
            return Err(StoreError::VersionUnsupported);
        }
        if binding.id != *id {
            return Err(StoreError::invalid());
        }
        Ok(Some(binding))
    }

    /// Writes the local binding atomically (0600 file in a 0700 `local/`
    /// dir), refusing a binding whose version this store does not write or
    /// whose serialization exceeds [`MAX_BINDING_BYTES`].
    pub fn put_binding(&self, binding: &LocalBinding) -> Result<(), StoreError> {
        self.put_binding_inner(binding, &AtomicReplace)
    }

    fn put_binding_inner(
        &self,
        binding: &LocalBinding,
        replacer: &dyn Replacer,
    ) -> Result<(), StoreError> {
        if binding.version != LocalBinding::VERSION {
            return Err(StoreError::VersionUnsupported);
        }
        self.ensure_local_dir()?;
        let path = self.binding_path(&binding.id);
        let data = serialize_binding(binding)?;
        private_file::stage_and_publish(&path, &data, replacer)
    }

    /// Removes the local binding for `id` if present; an absent binding is
    /// already the requested state.
    pub fn clear_binding(&self, id: &InvestigationId) -> Result<(), StoreError> {
        let path = self.binding_path(id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(private_file::io_error(error)),
        }
    }

    fn binding_path(&self, id: &InvestigationId) -> std::path::PathBuf {
        self.root
            .join("local")
            .join(format!("{}.json", id.as_str()))
    }

    fn ensure_local_dir(&self) -> Result<(), StoreError> {
        self.ensure_root()?;
        let dir = self.root.join("local");
        std::fs::create_dir_all(&dir).map_err(private_file::io_error)?;
        #[cfg(unix)]
        private_file::set_mode(&dir, 0o700)?;
        Ok(())
    }
}

/// Serializes the binding through a bounded writer, so an oversized binding
/// refuses mid-stream — `LimitExceeded` — instead of materializing past the
/// cap.
fn serialize_binding(binding: &LocalBinding) -> Result<Vec<u8>, StoreError> {
    let mut capped = BoundedWriter::new(Vec::new(), MAX_BINDING_BYTES);
    match serde_json::to_writer_pretty(&mut capped, binding) {
        Ok(()) => {}
        Err(error) if error.io_error_kind() == Some(std::io::ErrorKind::QuotaExceeded) => {
            return Err(StoreError::LimitExceeded);
        }
        Err(_) => return Err(StoreError::unavailable()),
    }
    Ok(capped.into_inner())
}
