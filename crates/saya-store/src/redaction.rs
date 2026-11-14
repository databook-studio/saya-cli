//! Re-export shim: `redact` lives in `saya-types` because two mid-layer
//! crates, `saya-agent` and `saya-store`, share it — a shared thing is a
//! contract in the leaf crate. Every `saya_store::redact` and
//! `crate::redaction::redact` call site resolves here.
pub use saya_types::redact;
