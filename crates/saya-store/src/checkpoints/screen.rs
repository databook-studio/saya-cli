//! The admission-screening seam for private before-images: called from
//! [`prepare_checkpoint`](super::ops) before any byte is written, it decides
//! whether the requested content may be stored as private recovery data at
//! all. This packet leaves the seam inert — it admits everything; the
//! screening rules (known credential paths, known-secret content,
//! database-export artifacts) arrive in the next packet and refuse through
//! [`ScreenRefusal`] with nothing ever touching disk.

use thiserror::Error;

/// Why admission screening refused content: private recovery data is never
/// stored when screening objects to it, and the refusal surfaces as
/// `CheckpointStoreError::Screened` with nothing written. No variants yet —
/// the screening rules are the next packet; until then the seam admits
/// every request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ScreenRefusal {}

/// The screening seam: decides whether `path` with `bytes` may be stored as
/// a private before-image. Inert for now — it admits everything.
pub(super) fn admit_content(_path: &str, _bytes: Option<&[u8]>) -> Result<(), ScreenRefusal> {
    Ok(())
}
