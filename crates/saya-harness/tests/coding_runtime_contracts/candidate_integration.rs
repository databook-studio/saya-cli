#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "candidate_integration/apply.rs"]
mod apply;
#[allow(dead_code)]
#[path = "candidate_integration/apply_support.rs"]
mod apply_support;
#[path = "candidate_integration/candidate.rs"]
mod candidate;
#[path = "candidate_integration/integrate.rs"]
mod integrate;
#[path = "candidate_integration/proof.rs"]
mod proof;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)] // shared test harness: this file uses only part of it
#[path = "candidate_integration/support.rs"]
mod support_active;

#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "managed_worktree_creation/create_support.rs"]
mod create_support;

#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "worktree_inventory_support.rs"]
mod support;
