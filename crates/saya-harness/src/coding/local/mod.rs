//! In-place coding roots and baselines for folders without a Git checkout.
//!
//! An in-place root is one canonical directory identified by a
//! domain-separated content address. Its baseline is an edit-time marker with
//! no inventory; tool edits are recorded in a before-image store and shell
//! edits are read from the root's Git status when it has one.

mod baseline;
pub mod before_image;
mod capture;
mod identity;
mod plan;
mod plan_error;
pub mod shell_changes;

pub use baseline::{JobBaseline, JobBaselineError};
pub use identity::{LocalRoot, LocalRootError, LocalRootId};
pub use plan::LocalBaseline;
pub use plan_error::LocalBaselineError;

#[cfg(all(test, unix))]
mod before_image_tests;
#[cfg(all(test, unix))]
mod capture_tests;
#[cfg(all(test, unix))]
mod identity_tests;
#[cfg(all(test, unix))]
mod plan_tests;
#[cfg(all(test, unix))]
mod test_support;
