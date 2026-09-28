#[cfg(unix)]
use crate::private_file::set_mode;
use crate::private_file::{bounded_read, io_error, stage_and_publish};
use crate::redaction::redact;
use crate::replace::{AtomicReplace, Replacer};
use crate::{RedactedSession, SessionHistoryPage, SessionHistoryQuery, SessionStore, StoreError};
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

    fn path(&self, id: &str) -> Result<PathBuf, StoreError> {
        if id.is_empty() || id.contains('/') || id.contains('\\') || id == "." || id == ".." {
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

    fn load_file(&self, path: &Path) -> Result<Option<RedactedSession>, StoreError> {
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

    async fn save_inner(
        &self,
        mut session: RedactedSession,
        replacer: &dyn Replacer,
    ) -> Result<(), StoreError> {
        self.ensure_root()?;
        for message in &mut session.messages {
            message.content = redact(&message.content);
        }
        for turn in &mut session.turns {
            turn.user = redact(&turn.user);
            turn.assistant = redact(&turn.assistant);
        }
        let path = self.path(&session.id)?;
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
        self.save_inner(session, replacer).await
    }
}

#[async_trait]
impl SessionStore for FsSessionStore {
    async fn save(&self, session: RedactedSession) -> Result<(), StoreError> {
        self.save_inner(session, &AtomicReplace).await
    }

    async fn load(&self, id: &str) -> Result<Option<RedactedSession>, StoreError> {
        self.load_file(&self.path(id)?)
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

fn stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default()
}
