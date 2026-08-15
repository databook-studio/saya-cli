//! Bounded discovery of `.saya/contracts/*.toml` — slice 6a.
//!
//! Discovers, validates and parses team contract files into typed claims held
//! in memory. **Nothing is imported into the store** — that is 6b, behind an
//! explicit command with a dry run. A parser exercisable without a write path
//! is one whose failure modes are cheap to explore.
//!
//! This is the first place the memory feature reads arbitrary files off disk;
//! see [`paths`] for the safety contract and the TOCTOU race we do not close.

mod bounds;
mod format;
mod pass;
mod paths;

#[cfg(test)]
#[path = "discover_tests.rs"]
mod tests;

use std::path::PathBuf;

use thiserror::Error;

use crate::contracts::args::QualifiedName;
use saya_types::ClaimPayload;

// This `pub(crate)` surface is the discovery API the 6b import command (and
// tests) will consume. Nothing outside this module references it yet in a
// non-test build, so the re-exports read as unused — they are not dead code,
// they are the boundary this slice exposes. Mirrors the pattern in
// `contracts/mod.rs`.
#[allow(unused_imports)]
pub(crate) use bounds::TruncationBound;
#[allow(unused_imports)]
pub(crate) use bounds::{MAX_BYTES_PER_FILE, MAX_FILES, MAX_TOTAL_BYTES};
#[allow(unused_imports)]
pub(crate) use format::ParsedContract;
#[allow(unused_imports)]
pub(crate) use pass::discover_contracts;
#[allow(unused_imports)]
pub(crate) use paths::RootError;
// The component-wise containment check the read side (6a) and the write side
// (6b) share. `pub(crate)` so `contracts::io::write` can reuse it instead of a
// second containment check — one path-safety primitive, both directions.
pub(crate) use paths::contains;

// Used only by tests to assert the claims-per-file cap; the bound itself is
// enforced in `format::parse_file`.
#[cfg(test)]
pub(crate) use bounds::MAX_CLAIMS_PER_FILE;

/// One successfully parsed contract file. `source` is **relative** to the
/// project root for display — an absolute path names a machine and a user,
/// and this value is rendered.
#[derive(Debug)]
pub(crate) struct DiscoveredContract {
    pub source: PathBuf,
    pub object: QualifiedName,
    pub claims: Vec<ClaimPayload>,
}

/// The outcome of a discovery pass.
#[derive(Debug)]
pub(crate) struct DiscoveryReport {
    pub contracts: Vec<DiscoveredContract>,
    pub rejected: Vec<(PathBuf, String)>,
    pub truncated_by: Option<&'static str>,
}

/// A hard failure of the pass itself — the root exists but is not a
/// directory, or cannot be read. A missing `.saya/contracts` is **not** an
/// error (the normal case) and returns an empty report. Per-file failures are
/// reported in [`DiscoveryReport::rejected`], never raised here.
#[derive(Debug, Error)]
pub(crate) enum DiscoveryError {
    #[error("{0}")]
    Root(#[from] RootError),
}

/// An empty report — the shape returned when `.saya/contracts` is absent.
pub(crate) fn empty_report() -> DiscoveryReport {
    DiscoveryReport {
        contracts: Vec::new(),
        rejected: Vec::new(),
        truncated_by: None,
    }
}
