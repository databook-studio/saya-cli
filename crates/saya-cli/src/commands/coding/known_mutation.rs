//! Private committed-byte provenance passed from the native mutation owner.

use saya_harness::workspace::MAX_IO_BYTES;
use saya_types::{Sha256Digest, WorkspacePath};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KnownWorkspaceMutationOperation {
    Write,
    Edit,
    /// A guarded removal: the digest is the removed file's pre-image.
    Delete,
}

/// Facts from one known successful guarded workspace publication. The caller
/// must derive them from publication bytes, never model output or a reread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KnownWorkspaceMutation {
    operation: KnownWorkspaceMutationOperation,
    path: WorkspacePath,
    sha256: Sha256Digest,
    size: u64,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum KnownWorkspaceMutationError {
    #[error("committed workspace path is invalid")]
    Path,
    #[error("committed workspace digest is invalid")]
    Digest,
    #[error("committed workspace size exceeds the guarded write bound")]
    Size,
}

impl KnownWorkspaceMutation {
    pub(crate) fn new(
        operation: KnownWorkspaceMutationOperation,
        path: String,
        sha256: String,
        size: u64,
    ) -> Result<Self, KnownWorkspaceMutationError> {
        if size > MAX_IO_BYTES as u64 {
            return Err(KnownWorkspaceMutationError::Size);
        }
        Ok(Self {
            operation,
            path: path
                .try_into()
                .map_err(|_| KnownWorkspaceMutationError::Path)?,
            sha256: Sha256Digest::parse(&sha256)
                .map_err(|_| KnownWorkspaceMutationError::Digest)?,
            size,
        })
    }

    pub(crate) fn operation(&self) -> KnownWorkspaceMutationOperation {
        self.operation
    }

    pub(crate) fn path(&self) -> &WorkspacePath {
        &self.path
    }

    pub(crate) fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }

    pub(crate) fn size(&self) -> u64 {
        self.size
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn digest() -> String {
        "a".repeat(64)
    }

    #[test]
    fn committed_facts_are_validated_before_they_cross_the_hook_boundary() {
        let mutation = KnownWorkspaceMutation::new(
            KnownWorkspaceMutationOperation::Edit,
            "src/lib.rs".into(),
            digest(),
            42,
        )
        .unwrap();
        assert_eq!(mutation.operation(), KnownWorkspaceMutationOperation::Edit);
        assert_eq!(mutation.path().as_str(), "src/lib.rs");
        assert_eq!(mutation.sha256().as_str(), digest());
        assert_eq!(mutation.size(), 42);
    }

    #[test]
    fn untrusted_or_oversized_facts_do_not_construct_a_known_mutation() {
        assert!(matches!(
            KnownWorkspaceMutation::new(
                KnownWorkspaceMutationOperation::Write,
                "../escape".into(),
                digest(),
                1,
            ),
            Err(KnownWorkspaceMutationError::Path)
        ));
        assert!(matches!(
            KnownWorkspaceMutation::new(
                KnownWorkspaceMutationOperation::Write,
                "src/lib.rs".into(),
                "not-a-digest".into(),
                1,
            ),
            Err(KnownWorkspaceMutationError::Digest)
        ));
        assert!(matches!(
            KnownWorkspaceMutation::new(
                KnownWorkspaceMutationOperation::Write,
                "src/lib.rs".into(),
                digest(),
                MAX_IO_BYTES as u64 + 1,
            ),
            Err(KnownWorkspaceMutationError::Size)
        ));
    }
}
