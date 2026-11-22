#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "guarded_applyback/apply_back.rs"]
mod apply_back;
#[path = "guarded_applyback/ownership.rs"]
mod ownership;
#[allow(dead_code)] // shared test harness: this target uses part of it
#[path = "guarded_applyback/support.rs"]
mod support_w11;

#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)] // shared test harness: this target uses part of it
#[path = "managed_worktree_creation/create_support.rs"]
mod create_support;

#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)] // shared test harness: this target uses part of it
#[path = "candidate_integration/apply_support.rs"]
mod apply_support;

#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)] // shared test harness: this target uses part of it
#[path = "worktree_inventory_support.rs"]
mod support;
