#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "applyback_recovery/admission.rs"]
mod admission;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "candidate_integration/apply_support.rs"]
mod apply_support;
#[path = "applyback_recovery/binding.rs"]
mod binding;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "managed_worktree_creation/create_support.rs"]
mod create_support;
#[path = "applyback_recovery/identity.rs"]
mod identity;
#[path = "applyback_recovery/recovery.rs"]
mod recovery;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "worktree_inventory_support.rs"]
mod support;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "guarded_applyback/support.rs"]
mod support_w11;
