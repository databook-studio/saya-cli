#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "managed_worktree_creation/create.rs"]
mod create;
#[path = "managed_worktree_creation/create_edges.rs"]
mod create_edges;
#[path = "managed_worktree_creation/create_retry.rs"]
mod create_retry;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)] // shared test harness: this file uses only part of it
#[path = "managed_worktree_creation/create_support.rs"]
mod create_support;
#[path = "managed_worktree_creation/ownership.rs"]
mod ownership;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "worktree_inventory_support.rs"]
mod support;
#[path = "managed_worktree_creation/worktree.rs"]
mod worktree;
