//! Contract import and export — slice 6b.
//!
//! Two operations across the boundary between this machine and a repository:
//! [`import_contracts`] turns discovered `.saya/contracts/*.toml` claims into
//! stored claims, and [`export_contracts`] writes stored confirmed claims back
//! to the discovered shape. Both are presentation-free typed operations the
//! adapter renders; neither carries the opaque profile identity, evidence, or
//! absolute paths across that boundary.
//!
//! `import` proposes discovered claims with `origin = TeamFile` and
//! `initial_status = Confirmed` (ADR 0002 §4: a reviewed team file enters
//! confirmed within its declared scope). The store admits `TeamFile` as
//! confirmable — see `ClaimOrigin::may_confirm_directly`. Conflicts with local
//! claims surface per ADR decision 3 at recall, not as silent overwrites here.

mod classify;
mod export;
mod import;
mod render;
mod write;

pub(crate) use classify::ImportVerdict;
pub(crate) use export::{ExportOutcome, export_contracts};
pub(crate) use import::import_contracts;

use thiserror::Error;

/// One discovered claim and the verdict it received.
#[derive(Debug, Clone)]
pub(crate) struct ImportClaimResult {
    pub source: std::path::PathBuf,
    pub object: String,
    pub verdict: ImportVerdict,
}

/// The outcome of an import pass.
#[derive(Debug, Clone, Default)]
pub(crate) struct ImportReport {
    pub added: Vec<ImportClaimResult>,
    pub duplicates: Vec<ImportClaimResult>,
    pub conflicts: Vec<ImportClaimResult>,
    pub stale: Vec<ImportClaimResult>,
    pub rejected: Vec<(std::path::PathBuf, String)>,
    pub truncated_by: Option<&'static str>,
}

/// Import-only argument errors. Payload-free: a malformed object name never
/// reaches the terminal.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum ArgError {
    #[error("a discovered object name is not a valid qualified name")]
    MalformedObject,
}
