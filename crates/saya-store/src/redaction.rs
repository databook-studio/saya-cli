//! Re-export shim: `redact` moved to `saya-types` (G4 — two mid-layer crates,
//! `saya-agent` and `saya-store`, share it, so the shared thing is a contract
//! in the leaf crate). The code and its tests moved verbatim; every
//! `saya_store::redact` and `crate::redaction::redact` call site compiles
//! unchanged.
pub use saya_types::redact;
