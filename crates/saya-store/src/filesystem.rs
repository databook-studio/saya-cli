#[cfg(unix)]
use crate::private_file::set_mode;
use crate::private_file::{bounded_read, io_error, stage_and_publish};
use crate::replace::{AtomicReplace, Replacer};
use crate::session_lifecycle::tombstone;
use crate::{
    RedactedSession, SessionHistoryPage, SessionHistoryQuery, SessionStore, StoreError,
    StoredContinuation,
};
use async_trait::async_trait;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Maximum serialized session size kept on disk. This bounds both reads and
/// writes so a malformed or untrusted record cannot force an unbounded
/// allocation during resume.
pub const MAX_SESSION_BYTES: usize = 4 << 20;

#[derive(Clone)]
pub struct FsSessionStore {
    root: PathBuf,
}

impl FsSessionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The sessions root: the parent of every `<id>.json` record and of
    /// each per-session state directory. Crate-visible so the lifecycle
    /// modules derive tombstone and cleanup paths from the same root.
    pub(crate) fn sessions_root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn path(&self, id: &str) -> Result<PathBuf, StoreError> {
        if !is_valid_id(id) {
            return Err(StoreError::unavailable());
        }
        Ok(self.root.join(format!("{id}.json")))
    }

    fn ensure_root(&self) -> Result<(), StoreError> {
        fs::create_dir_all(&self.root).map_err(io_error)?;
        #[cfg(unix)]
        {
            set_mode(&self.root, 0o700)?;
        }
        Ok(())
    }

    pub(crate) fn load_file(&self, path: &Path) -> Result<Option<RedactedSession>, StoreError> {
        let bytes = match bounded_read(path, MAX_SESSION_BYTES)? {
            Some(value) => value,
            None => return Ok(None),
        };
        let content = match String::from_utf8(bytes) {
            Ok(value) => value,
            Err(_) => return self.quarantine(path),
        };
        match serde_json::from_str(&content) {
            Ok(session) => Ok(Some(session)),
            Err(_) => self.quarantine(path),
        }
    }

    fn quarantine(&self, path: &Path) -> Result<Option<RedactedSession>, StoreError> {
        let corrupt = path.with_extension(format!("corrupt-{}", stamp()));
        let _ = fs::rename(path, corrupt);
        Ok(None)
    }

    /// Whether the session already on disk carries a continuation this
    /// binary does not understand. Read plainly, never quarantining: a
    /// missing, unreadable, or unparsable file cannot answer the question
    /// and never triggers the guard — the load path quarantines such files,
    /// and refusing saves over them would strand the session. A `null`
    /// continuation key is absence, not an unknown record.
    fn existing_continuation_is_unsupported(&self, path: &Path) -> bool {
        let bytes = match bounded_read(path, MAX_SESSION_BYTES) {
            Ok(Some(value)) => value,
            _ => return false,
        };
        let Ok(content) = String::from_utf8(bytes) else {
            return false;
        };
        let Ok(record) = serde_json::from_str::<serde_json::Value>(&content) else {
            return false;
        };
        let Some(continuation) = record.get("continuation") else {
            return false;
        };
        if continuation.is_null() {
            return false;
        }
        matches!(
            StoredContinuation::from_value(continuation.clone()),
            StoredContinuation::Unsupported(_)
        )
    }

    pub(crate) fn save_inner(
        &self,
        mut session: RedactedSession,
        replacer: &dyn Replacer,
    ) -> Result<(), StoreError> {
        self.ensure_root()?;
        // Continuation handling at the store boundary: the shared prepare
        // step (redaction, then continuation prose redaction and
        // revalidation — see `session_lifecycle`) runs before any disk
        // write, so a save and the capacity report cannot drift.
        crate::session_lifecycle::prepare_session_for_persist(&mut session)?;
        let path = self.path(&session.id)?;
        // A session with a pending-deletion tombstone cannot be resurrected
        // by a save; the previous bytes stay untouched.
        if tombstone::exists(&self.root, &session.id)? {
            return Err(StoreError::Conflict);
        }
        if !matches!(
            session.continuation,
            Some(StoredContinuation::Unsupported(_))
        ) && self.existing_continuation_is_unsupported(&path)
        {
            return Err(StoreError::UnsupportedContinuation);
        }
        // Bounded serialization: the pretty JSON streams into a writer that
        // refuses at the ceiling, so an oversized session fails mid-stream —
        // never after a full `Vec` materialization — and the publish below
        // (and the old record) is never reached.
        let mut capped = crate::bounded::BoundedWriter::new(Vec::new(), MAX_SESSION_BYTES);
        match serde_json::to_writer_pretty(&mut capped, &session) {
            Ok(()) => {}
            Err(error) if error.io_error_kind() == Some(std::io::ErrorKind::QuotaExceeded) => {
                return Err(StoreError::LimitExceeded);
            }
            Err(_) => return Err(StoreError::unavailable()),
        }
        let data = capped.into_inner();
        // Private staging beside the target plus atomic publish: the rename
        // either installs the complete new file or leaves the existing
        // target untouched — it never truncates the target before failing,
        // unlike the old Windows copy-then-remove.
        stage_and_publish(&path, &data, replacer)
    }

    /// Test-only save through an injected [`Replacer`]: proves a failed
    /// publish preserves the last good session on any platform.
    #[cfg(test)]
    pub(crate) async fn save_with_replacer(
        &self,
        session: RedactedSession,
        replacer: &dyn Replacer,
    ) -> Result<(), StoreError> {
        self.save_inner(session, replacer)
    }
}

#[async_trait]
impl SessionStore for FsSessionStore {
    async fn save(&self, session: RedactedSession) -> Result<(), StoreError> {
        self.save_inner(session, &AtomicReplace)
    }

    async fn load(&self, id: &str) -> Result<Option<RedactedSession>, StoreError> {
        let path = self.path(id)?;
        // A tombstoned session is invisible at once — and its record is
        // never quarantined by this read.
        if tombstone::exists(&self.root, id)? {
            return Ok(None);
        }
        self.load_file(&path)
    }

    async fn most_recent(&self) -> Result<Option<RedactedSession>, StoreError> {
        let entries = match fs::read_dir(&self.root) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(error)),
        };
        let mut paths = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .collect::<Vec<_>>();
        paths.sort_by_key(|entry| entry.metadata().and_then(|meta| meta.modified()).ok());
        for entry in paths.into_iter().rev() {
            if let Some(id) = entry.path().file_stem().and_then(|stem| stem.to_str())
                && tombstone::exists(&self.root, id)?
            {
                continue;
            }
            if let Some(session) = self.load_file(&entry.path())? {
                return Ok(Some(session));
            }
        }
        Ok(None)
    }

    async fn history(&self, query: SessionHistoryQuery) -> Result<SessionHistoryPage, StoreError> {
        crate::history::list(&self.root, &query)
    }
}

/// Whether `id` is a valid session id: non-empty, no path separators, and
/// never the relative-navigation names. One rule for every derivation from
/// an id — the session file path, the per-session state dir, tombstones.
pub(crate) fn is_valid_id(id: &str) -> bool {
    !(id.is_empty() || id.contains('/') || id.contains('\\') || id == "." || id == "..")
}

pub(crate) fn stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default()
}
