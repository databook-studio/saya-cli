//! Bounds for team-contract file discovery (plan §12 Phase 6).
//!
//! Each bound exists because a `.saya/contracts` directory is attacker-
//! influenceable in a shared repo. Exceeding any one stops the pass and is
//! reported via [`TruncationBound`] — never silently truncated (plan §12's
//! "no silent caps" rule: truncating quietly reads as "we looked at
//! everything" when we did not).

/// Maximum number of files discovered in a single pass.
pub(crate) const MAX_FILES: usize = 32;
/// Maximum bytes read from a single file. One contract is small; a large one
/// is a mistake or an attack.
pub(crate) const MAX_BYTES_PER_FILE: usize = 64 * 1024;
/// Maximum total bytes read across the whole pass, regardless of file count.
pub(crate) const MAX_TOTAL_BYTES: usize = 1024 * 1024;
/// Maximum claims accepted from a single file. Matches the store's per-object
/// cap.
pub(crate) const MAX_CLAIMS_PER_FILE: usize = 128;

/// Which bound stopped a discovery pass. The string is payload-free and
/// rendered to the user as the reason discovery was truncated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum TruncationBound {
    Files,
    BytesPerFile,
    TotalBytes,
    ClaimsPerFile,
}

impl TruncationBound {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::BytesPerFile => "bytes-per-file",
            Self::TotalBytes => "total-bytes",
            Self::ClaimsPerFile => "claims-per-file",
        }
    }
}
