//! W04b: public-boundary tests of `materialize_snapshot` (authored, not
//! executed). Registers the shared W02 support fixtures verbatim and the
//! materialize acceptance, refusal, and rollback modules.
#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "snapshot_materialization/materialize.rs"]
mod materialize;
#[path = "snapshot_materialization/materialize_cleanup.rs"]
mod materialize_cleanup;
#[path = "snapshot_materialization/materialize_gitfile.rs"]
mod materialize_gitfile;
#[path = "snapshot_materialization/materialize_refusals.rs"]
mod materialize_refusals;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "worktree_inventory_support.rs"]
mod support;
