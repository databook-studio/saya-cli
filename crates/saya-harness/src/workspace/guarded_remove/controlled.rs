//! Model-facing removal keeps precommit refusal distinct from an unknown unlink.
#[cfg(unix)]
use crate::HarnessError;
use crate::workspace::Workspace;
use saya_agent::CancellationToken;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RemovalRefusal {
    #[error("invalid deletion path or digest")]
    Invalid,
    #[error("deletion target is unavailable or not a regular contained file")]
    Target,
    #[error("deletion target changed since it was observed")]
    Changed,
    #[error("deletion target exceeds the bounded file size")]
    TooLarge,
    #[error("deletion target is private or contains screened data")]
    Private,
    #[error("deletion was cancelled before unlink")]
    Cancelled,
    #[error("deletion deadline reached before unlink")]
    Deadline,
    #[error("deletion is unavailable on this platform")]
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ControlledRemoveError {
    #[error("workspace deletion refused: {0}")]
    NoEffect(RemovalRefusal),
    #[error("workspace deletion outcome could not be confirmed")]
    Unknown,
}

impl ControlledRemoveError {
    #[cfg(unix)]
    fn before(error: HarnessError) -> Self {
        let category = match error {
            HarnessError::ContentChanged { .. } | HarnessError::IdentityChanged { .. } => {
                RemovalRefusal::Changed
            }
            HarnessError::BoundsExceeded { .. } => RemovalRefusal::TooLarge,
            _ => RemovalRefusal::Target,
        };
        Self::NoEffect(category)
    }
}

pub(in crate::workspace) fn check(
    token: &CancellationToken,
    deadline: Instant,
) -> Result<(), ControlledRemoveError> {
    if token.is_cancelled() {
        return Err(ControlledRemoveError::NoEffect(RemovalRefusal::Cancelled));
    }
    if Instant::now() >= deadline {
        return Err(ControlledRemoveError::NoEffect(RemovalRefusal::Deadline));
    }
    Ok(())
}

/// A held verified snapshot. It stays in one blocking operation through commit;
/// Debug deliberately omits private path, digest and content.
pub struct PreparedRemoval {
    #[cfg(unix)]
    anchor: crate::workspace::anchor::Anchor,
    #[cfg(unix)]
    file: std::fs::File,
    #[cfg(unix)]
    _gate: std::sync::MutexGuard<'static, ()>,
    #[cfg(unix)]
    token: CancellationToken,
    #[cfg(unix)]
    deadline: Instant,
}
impl std::fmt::Debug for PreparedRemoval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedRemoval").finish_non_exhaustive()
    }
}
impl Workspace {
    /// Prepares one model deletion under the same cooperative publication gate.
    /// Successful preparation grants no authority; the caller supplies an owned
    /// workspace and must keep its independent command/writer lease through commit.
    pub fn prepare_guarded_remove(
        &self,
        path: &str,
        expected_digest: &str,
        token: CancellationToken,
        deadline: Instant,
    ) -> Result<PreparedRemoval, ControlledRemoveError> {
        use ControlledRemoveError::NoEffect;
        check(&token, deadline)?;
        if path.len() > 512
            || saya_types::WorkspacePath::try_from(path.to_owned()).is_err()
            || saya_types::Sha256Digest::parse(expected_digest).is_err()
        {
            return Err(NoEffect(RemovalRefusal::Invalid));
        }
        if crate::coding::snapshot::sensitive_path(path)
            || saya_store::screen_private_content(path, None).is_err()
        {
            return Err(NoEffect(RemovalRefusal::Private));
        }
        #[cfg(not(unix))]
        return Err(NoEffect(RemovalRefusal::Unsupported));
        #[cfg(unix)]
        {
            use std::io::Read;
            let gate = super::super::commit_gate::hold_until(&token, deadline)?;
            let anchor = self
                .anchor(path, false)
                .map_err(ControlledRemoveError::before)?;
            let stat = super::super::anchored::final_stat(&anchor, "remove workspace file", path)
                .map_err(ControlledRemoveError::before)?;
            if !stat.is_file() {
                return Err(NoEffect(RemovalRefusal::Target));
            }
            let cap = super::super::contain::MAX_IO_BYTES;
            if stat.len() > cap as u64 {
                return Err(NoEffect(RemovalRefusal::TooLarge));
            }
            let mut file = anchor
                .open_verified(stat.identity(), path)
                .map_err(ControlledRemoveError::before)?;
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 64 * 1024];
            loop {
                check(&token, deadline)?;
                let read = file
                    .read(&mut chunk)
                    .map_err(|_| NoEffect(RemovalRefusal::Target))?;
                if read == 0 {
                    break;
                }
                if bytes.len().saturating_add(read) > cap {
                    return Err(NoEffect(RemovalRefusal::TooLarge));
                }
                bytes.extend_from_slice(&chunk[..read]);
            }
            Self::digest_checked(path, Some(expected_digest), &bytes)
                .map_err(ControlledRemoveError::before)?;
            saya_store::screen_private_content(path, Some(&bytes))
                .map_err(|_| NoEffect(RemovalRefusal::Private))?;
            drop(bytes);
            Ok(PreparedRemoval {
                anchor,
                file,
                _gate: gate,
                token,
                deadline,
            })
        }
    }
}

impl PreparedRemoval {
    /// Performs the one unlink attempt; an opaque attempt failure is uncertain.
    pub fn commit(self) -> Result<(), ControlledRemoveError> {
        #[cfg(not(unix))]
        return Err(ControlledRemoveError::NoEffect(RemovalRefusal::Unsupported));
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            check(&self.token, self.deadline)?;
            let opened = self
                .file
                .metadata()
                .map_err(|_| ControlledRemoveError::NoEffect(RemovalRefusal::Target))?;
            let named = self
                .anchor
                .stat_final()
                .map_err(|_| ControlledRemoveError::NoEffect(RemovalRefusal::Changed))?;
            if !named.is_file()
                || named.identity() != (opened.dev(), opened.ino())
                || named.len() != opened.len()
                || self.anchor.stat().map(|stat| stat.len()) != Some(opened.len())
            {
                return Err(ControlledRemoveError::NoEffect(RemovalRefusal::Changed));
            }
            // This is a cooperative writer guard, not atomic digest-CAS against
            // an OS writer changing the entry or same-inode bytes after this check.
            check(&self.token, self.deadline)?;
            self.anchor
                .unlink_final()
                .map_err(|_| ControlledRemoveError::Unknown)
        }
    }
}

#[cfg(test)]
mod tests;
