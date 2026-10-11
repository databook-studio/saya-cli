//! W04b: public-boundary tests of `materialize_snapshot` (authored, not
//! executed). Registers the shared W02 support fixtures verbatim and the
//! materialize acceptance, refusal, and rollback modules.
#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "w04b/materialize.rs"]
mod materialize;
#[path = "w04b/materialize_cleanup.rs"]
mod materialize_cleanup;
#[path = "w04b/materialize_gitfile.rs"]
mod materialize_gitfile;
#[path = "w04b/materialize_refusals.rs"]
mod materialize_refusals;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "w02_support.rs"]
mod support;
