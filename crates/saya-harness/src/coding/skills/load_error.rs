//! Bounded refusals for skill body loading.

use thiserror::Error;

/// Bounded load refusals. Diagnostics intentionally omit the skill text and
/// any filesystem path: both are untrusted input, and a refusal must not
/// leak the bytes it declined to admit.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SkillLoadError {
    /// The entry's root index does not address one of the supplied roots.
    #[error("skill root index is out of range for the supplied roots")]
    UnknownRoot,
    /// The skill source file was absent when it was reopened.
    #[error("skill source file is missing")]
    Missing,
    /// The skill source file is a symlink; only regular files are admitted.
    #[error("skill source file is a symlink")]
    Symlink,
    /// The skill source file is not a regular file.
    #[error("skill source file is not a regular file")]
    NotRegular,
    /// The reopened file exceeded a byte bound. That bound may apply to the
    /// skill body alone or to the whole file read (frontmatter plus body).
    #[error("skill file exceeded an applicable byte bound ({max})")]
    TooLarge {
        /// The exceeded byte bound, which may apply to the body alone or to
        /// the whole file read.
        max: usize,
    },
    /// The reopened frontmatter no longer matches the metadata recorded at
    /// discovery, or no longer parses as the frontmatter that was indexed.
    #[error("skill metadata changed since discovery")]
    MetadataChanged,
    /// The body is not valid UTF-8 or contains a NUL byte.
    #[error("skill body is not valid bounded text")]
    InvalidText,
    /// Loading fails closed on platforms without the anchored Unix seam.
    #[error("skill body loading is unsupported on this platform")]
    Unsupported,
    /// The workspace refused the anchored open, or the verified handle was
    /// unreadable.
    #[error("skill source file could not be read")]
    Unreadable,
}
