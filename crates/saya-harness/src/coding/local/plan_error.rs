//! The local baseline's typed failures. Paths are named only in the
//! per-file bound error; every other variant is path-free.

use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LocalBaselineError {
    #[error("the in-place folder contains an unsafe path")]
    UnsafePath,
    #[error("the in-place baseline was cancelled")]
    Cancelled,
    #[error("the in-place folder has {measured} files and the baseline allows {limit}")]
    FileCountExceeded { measured: usize, limit: usize },
    #[error("the in-place folder totals {measured} bytes and the baseline allows {limit}")]
    TotalBytesExceeded { measured: u64, limit: u64 },
    #[error("the file `{path}` is {measured} bytes and the baseline allows {limit} per file")]
    FileBytesExceeded {
        path: String,
        measured: u64,
        limit: u64,
    },
    #[error("the in-place baseline manifest is {measured} bytes and allows {limit}")]
    ManifestBytesExceeded { measured: usize, limit: usize },
    #[error("the in-place baseline manifest is malformed")]
    Malformed,
    #[error("the in-place baseline digest does not match its manifest")]
    DigestMismatch,
    #[error("the in-place baseline manifest could not be serialized")]
    Serialization,
    #[error("the in-place folder could not be read")]
    Io,
    #[error("the in-place baseline is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("the in-place baseline root path cannot be identified")]
    UnsupportedRoot,
}
