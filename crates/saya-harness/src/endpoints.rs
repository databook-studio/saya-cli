//! The role every run has — a mirror of `saya-config::ORCHESTRATOR_ROLE`.
//!
//! The generated run configuration (`state/config/{config,connections}.toml`),
//! its writer, and the input shapes it consumed all lived here. Their only
//! production caller was the nested-saya re-entry, which is deleted, so they
//! went with it; the one role name below is what the composition root still
//! names.

/// The role every run has; a mirror of `saya-config::ORCHESTRATOR_ROLE`
/// (see the module docs) — the pool's entry of this name is the fallback.
pub const ORCHESTRATOR_ROLE: &str = "orchestrator";
