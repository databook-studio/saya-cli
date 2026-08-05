use crate::redaction::redact;
use crate::{RedactedSession, SessionStore, SessionSummary, StoreError};
use async_trait::async_trait;
use std::{
    fs,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

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
        let content = match fs::read_to_string(path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(error)),
        };
        match serde_json::from_str(&content) {
            Ok(session) => Ok(Some(session)),
            Err(_) => {
                let corrupt = path.with_extension(format!("corrupt-{}", stamp()));
                let _ = fs::rename(path, corrupt);
                Ok(None)
            }
        }
    }
}

#[async_trait]
impl SessionStore for FsSessionStore {
    async fn save(&self, mut session: RedactedSession) -> Result<(), StoreError> {
        self.ensure_root()?;
        for message in &mut session.messages {
            message.content = redact(&message.content);
        }
        for turn in &mut session.turns {
            turn.user = redact(&turn.user);
            turn.assistant = redact(&turn.assistant);
        }
        let path = self.path(&session.id)?;
        let temp = path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(&session).map_err(|_| StoreError::unavailable())?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp)
            .map_err(io_error)?;
        file.write_all(&data).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        #[cfg(unix)]
        {
            set_mode(&temp, 0o600)?;
        }
        replace_file(&temp, &path)
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

    async fn history(&self) -> Result<Vec<SessionSummary>, StoreError> {
        crate::history::list(&self.root)
    }
}

fn replace_file(temp: &Path, target: &Path) -> Result<(), StoreError> {
    #[cfg(windows)]
    {
        fs::copy(temp, target).map_err(io_error)?;
        fs::remove_file(temp).map_err(io_error)
    }
    #[cfg(not(windows))]
    {
        fs::rename(temp, target).map_err(io_error)
    }
}

fn stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default()
}
fn io_error(_: std::io::Error) -> StoreError {
    StoreError::unavailable()
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(io_error)
}
