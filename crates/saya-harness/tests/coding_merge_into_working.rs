#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "coding_merge_into_working/cases.rs"]
mod cases;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "coding_runtime_contracts/managed_worktree_creation/create_support.rs"]
mod create_support;
#[allow(dead_code)]
#[path = "coding_merge_into_working/fixture.rs"]
mod fixture;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "coding_runtime_contracts/worktree_inventory_support.rs"]
mod support;
