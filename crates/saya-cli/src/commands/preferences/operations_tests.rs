//! Unit tests for the shared project-preference operations (M04a).
//!
//! The contract under test: preferences are explicit, bounded, per-worktree,
//! secret-free, never created by the model, and managed without a database
//! profile. The regressions guarded: accepting an unvalidated id (any
//! malformed id must be refused before the store is touched), and any
//! cross-worktree leak (another worktree cannot see, edit, or forget a
//! preference it does not hold). No test-only seam: every test drives the
//! same `pub(crate)` operations the CLI subcommand renders and the slash
//! packet will reuse.

use super::operations::{
    add_preference, edit_preference, forget_all_preferences, forget_preference, list_preferences,
};
use super::{PreferenceOutcome, PreferenceRefusal, render};
use crate::interactive::session::continuation::now_unix_ms;
use saya_store::SqliteStateStore;
use saya_types::{
    PROJECT_PREFERENCE_VERSION, PreferenceId, PreferenceSource, ProjectPreference, WorktreeId,
};

#[path = "operations_tests/lifecycle.rs"]
mod lifecycle;
#[path = "operations_tests/limits.rs"]
mod limits;
#[path = "operations_tests/rendering.rs"]
mod rendering;
#[path = "operations_tests/support.rs"]
mod support;
#[path = "operations_tests/worktree_isolation.rs"]
mod worktree_isolation;
