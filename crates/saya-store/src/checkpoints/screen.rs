//! The admission-screening seam for private before-images: called from
//! [`prepare_checkpoint`](super::ops) before any byte is written, it decides
//! whether the requested content may be stored as private recovery data at
//! all. Three refusal classes: a credential-file path, known-secret
//! content, and a database-export artifact. Refusal is refuse-only — the
//! before-image preserves exact bytes (a redacted copy cannot be advertised
//! as a faithful restore), so unsafe content is refused, never scrubbed,
//! and nothing is ever written.
//!
//! This screening is a known-shape screen, NOT a proof that arbitrary
//! source contains no sensitive information: an ordinary file holding a
//! secret under an unrecognized name or shape still passes. A refusal
//! surfaces to the caller as "recovery unavailable for this edit" (later
//! packets); this seam only refuses. The tables live beside it: the
//! `rules` submodule holds the path tables, `content` the byte inspection.

mod content;
mod rules;

use thiserror::Error;

/// Why admission screening refused content: private recovery data is never
/// stored when screening objects to it, and the refusal surfaces as
/// `CheckpointStoreError::Screened` with nothing written. The display names
/// only the class — never the path or the content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ScreenRefusal {
    /// The edit targets a path that looks like a credential file.
    #[error("the edit targets a credential file path")]
    CredentialPath,
    /// The bytes carry a known-secret shape: credential text the shared
    /// redactor would rewrite, or a private-key block header in non-UTF-8
    /// bytes.
    #[error("the edit content contains a known secret shape")]
    SecretContent,
    /// The file is a database-export artifact, by name, extension, or
    /// leading magic.
    #[error("the edit content is a database-export artifact")]
    DatabaseExport,
}

/// The screening seam: decides whether `path` with `bytes` may be stored as
/// a private before-image. `bytes: None` means the file did not exist, so
/// only the path rules apply to a create. Order is fixed: the cheap path
/// rules first, the whole-payload content scan last because it is the
/// expensive one.
pub(super) fn admit_content(path: &str, bytes: Option<&[u8]>) -> Result<(), ScreenRefusal> {
    if rules::refuses_credential_path(path) {
        return Err(ScreenRefusal::CredentialPath);
    }
    if rules::is_database_export_path(path) || bytes.is_some_and(content::is_database_export) {
        return Err(ScreenRefusal::DatabaseExport);
    }
    if bytes.is_some_and(content::contains_secret) {
        return Err(ScreenRefusal::SecretContent);
    }
    Ok(())
}
