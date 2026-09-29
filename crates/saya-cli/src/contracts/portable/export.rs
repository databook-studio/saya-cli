//! The `contracts export` operation (B2c invariant 1): a profile's Active
//! claims as a `saya.context` v1 file, written atomically and privately.
//!
//! Identity is stripped by [`PortablePayload::from_claim`] — the file carries
//! what a claim *is*, never which machine it came from. A claim that cannot be
//! made portable is counted and reported, never silently dropped. The write
//! refuses a symlink or directory target on every path (even `--overwrite`,
//! whose atomic rename would replace the link itself, not the file it points
//! at) and stages to a private temp beside the target before one rename, so a
//! failure leaves any existing file byte-for-byte unchanged.

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use saya_store::{KnowledgeItem, KnowledgeItemStore, SqliteStateStore};
use saya_types::{
    CONTEXT_FORMAT, CONTEXT_FORMAT_VERSION, ContextDocumentV1, ContextItem, KnowledgeState,
    MAX_ITEMS, PortableObject, PortablePayload, ProfileIdentity,
};

use super::PortableError;

/// What one export produced: the file it wrote, the claim counts per kind, and
/// the claims that could not be made portable (reported, never dropped).
pub(crate) struct ExportOutcome {
    pub path: PathBuf,
    pub total: usize,
    /// Serialized-JSON `ClaimPayload::kind()` → count of Active claims.
    pub kinds: Vec<(String, usize)>,
    pub skipped: usize,
}

/// Exports `identity`'s Active claims to `path` as one context document.
pub(crate) async fn export(
    store: &SqliteStateStore,
    identity: &ProfileIdentity,
    path: &Path,
    overwrite: bool,
) -> Result<ExportOutcome, PortableError> {
    let items = store.knowledge_for_profile(identity).await?;
    let active: Vec<&KnowledgeItem> = items
        .iter()
        .filter(|item| item.state == KnowledgeState::Active)
        .collect();
    if active.len() > MAX_ITEMS {
        return Err(PortableError::TooManyClaims(active.len()));
    }
    let mut document_items = Vec::with_capacity(active.len());
    let mut kinds = std::collections::BTreeMap::new();
    let mut skipped = 0usize;
    for item in &active {
        let Ok(payload) = PortablePayload::from_claim(&item.value) else {
            skipped += 1;
            continue;
        };
        *kinds.entry(item.value.kind()).or_insert(0) += 1;
        document_items.push(ContextItem {
            object: PortableObject {
                catalog: Some(item.object.catalog().to_string()),
                schema: Some(item.object.schema().to_string()),
                name: item.object.object().to_string(),
                kind: item.object.kind(),
            },
            payload,
            origin_note: None,
        });
    }
    let document_count = document_items.len();
    let document = ContextDocumentV1 {
        format: CONTEXT_FORMAT.to_string(),
        version: CONTEXT_FORMAT_VERSION,
        exported_unix_ms: super::super::now_unix_ms(),
        items: document_items,
    };
    let bytes = document.to_json_pretty()?;
    write_private(path, bytes.as_bytes(), overwrite)?;
    Ok(ExportOutcome {
        path: path.to_path_buf(),
        total: document_count,
        kinds: kinds
            .into_iter()
            .map(|(kind, count)| (kind.to_string(), count))
            .collect(),
        skipped,
    })
}

static WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Stages `bytes` at a private temp beside `path` (created new, mode 0600 on
/// unix, fsynced) and renames it into place — the one atomic publish the
/// session and investigation stores use. The temp is removed on any failure.
fn write_private(path: &Path, bytes: &[u8], overwrite: bool) -> Result<(), PortableError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(PortableError::IsSymlink);
        }
        if metadata.is_dir() {
            return Err(PortableError::IsDirectory);
        }
        if !overwrite {
            return Err(PortableError::Exists);
        }
    }
    let sequence = WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp = path.with_file_name(format!(
        ".saya-export.{}.{}.tmp",
        std::process::id(),
        sequence
    ));
    let staged = (|| {
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp).map_err(PortableError::Write)?;
        file.write_all(bytes).map_err(PortableError::Write)?;
        file.sync_all().map_err(PortableError::Write)?;
        fs::rename(&temp, path).map_err(PortableError::Write)
    })();
    if staged.is_err() {
        let _ = fs::remove_file(&temp);
    }
    staged
}
