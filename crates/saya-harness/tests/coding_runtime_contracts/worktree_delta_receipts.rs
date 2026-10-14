#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "worktree_delta_receipts/delta.rs"]
mod delta;
#[path = "worktree_delta_receipts/delta_edges.rs"]
mod delta_edges;
#[path = "worktree_delta_receipts/delta_output.rs"]
mod delta_output;
#[path = "worktree_delta_receipts/paths.rs"]
mod paths;
#[path = "worktree_delta_receipts/receipt.rs"]
mod receipt;

#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "managed_worktree_creation/create_support.rs"]
mod create_support;

#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "worktree_inventory_support.rs"]
mod support;
