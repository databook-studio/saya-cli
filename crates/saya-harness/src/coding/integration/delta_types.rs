use saya_types::Sha256Digest;
use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PathDelta {
    path: String,
    kind: DeltaKind,
    bytes: u64,
    content_sha256: Option<Sha256Digest>,
}

impl PathDelta {
    pub(super) fn new(
        path: String,
        kind: DeltaKind,
        bytes: u64,
        content_sha256: Option<Sha256Digest>,
    ) -> Self {
        Self {
            path,
            kind,
            bytes,
            content_sha256,
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn kind(&self) -> DeltaKind {
        self.kind
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn content_sha256(&self) -> Option<&Sha256Digest> {
        self.content_sha256.as_ref()
    }
}

impl fmt::Debug for PathDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PathDelta")
            .field("kind", &self.kind)
            .field("bytes", &self.bytes)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct CheckoutDelta {
    entries: Vec<PathDelta>,
}

impl CheckoutDelta {
    pub(super) fn new(mut entries: Vec<PathDelta>) -> Self {
        entries.sort_unstable_by(|a, b| a.path.cmp(&b.path));
        Self { entries }
    }

    pub fn entries(&self) -> &[PathDelta] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn total_bytes(&self) -> u64 {
        self.entries
            .iter()
            .fold(0u64, |acc, entry| acc.saturating_add(entry.bytes))
    }
}

impl fmt::Debug for CheckoutDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckoutDelta")
            .field("count", &self.entries.len())
            .finish()
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeltaError {
    #[error("the delta computation was cancelled")]
    Cancelled,
    #[error("the delta computation exceeded its deadline")]
    DeadlineExceeded,
    #[error("the checkout holds a symlink or special file")]
    Symlink,
    #[error("the checkout holds a nested repository marker")]
    NestedRepository,
    #[error("the checkout holds an unsafe path")]
    UnsafePath,
    #[error("the checkout holds paths that differ only by case")]
    CaseFoldCollision,
    #[error("the checkout holds too many files")]
    TooManyFiles,
    #[error("a checkout file exceeds the size bound")]
    FileTooLarge,
    #[error("the checkout exceeds the total size bound")]
    TooLarge,
    #[error("the verified snapshot does not match the checkout")]
    SnapshotMismatch,
    #[error("the checkout's output root exceeds its bound")]
    OutputRootTooLarge,
    #[error("the checkout could not be read")]
    Io,
}

pub const DEFAULT_DELTA_DEADLINE: Duration = Duration::from_secs(30);

pub(super) struct LiveFile {
    pub(super) path: String,
    pub(super) executable: bool,
    pub(super) bytes: u64,
    pub(super) sha256: Sha256Digest,
}
