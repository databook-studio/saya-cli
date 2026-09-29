//! Portable reviewed business context (B2c, ADR 0006): the typed operations
//! behind `contracts export|import|import-dbt`.
//!
//! [`export`] writes a profile's Active claims as a `saya.context` v1 file
//! with every machine-bound identity stripped; [`import_document`] validates a
//! whole document, maps its logical objects onto one profile's schema, and
//! files what resolved as Pending through the store's transactional batch —
//! conflicts and unresolvable objects are reported, never written, and an
//! imported claim never arrives with authority. The mapping rules live in
//! [`map`]; both operations carry typed, payload-free errors so no untrusted
//! file content can reach a message.

mod export;
mod import;
mod map;
mod resolve;

#[cfg(test)]
mod tests;

pub(crate) use export::{ExportOutcome, export};
pub(crate) use import::{ImportOutcome, import_document, read_context};
pub(crate) use map::{Unavailability, map_items};

use saya_store::KnowledgeStoreError;
use saya_types::ContextError;
use thiserror::Error;

/// Why a portable export/import refused. Payload-free: no file content is ever
/// echoed except the export/import paths, which are the user's own arguments.
#[derive(Debug, Error)]
#[non_exhaustive]
pub(crate) enum PortableError {
    #[error("the context file is {0} bytes, over the 1 MiB limit")]
    Oversize(usize),
    #[error("could not read the context file: {0}")]
    Read(#[from] std::io::Error),
    #[error("{0}")]
    Document(#[from] ContextError),
    #[error("the export path already exists; pass --overwrite to replace it")]
    Exists,
    #[error("the export path is a directory")]
    IsDirectory,
    #[error("the export path is a symbolic link")]
    IsSymlink,
    #[error("could not write the context file: {0}")]
    Write(std::io::Error),
    #[error("the profile has {0} active claims, over the 500-item document limit")]
    TooManyClaims(usize),
    #[error("the store refused the import: {0}")]
    Store(#[from] KnowledgeStoreError),
}
