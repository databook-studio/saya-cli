//! Value-free provenance: which layer supplied each user-facing setting.
//!
//! Recorded inside the single merge funnel (`layers.rs`) where a layer
//! assigns — never a parallel merge. A secret appears only as the redacted
//! label of its reference: no values, no paths.

mod defaults;
mod keys;
mod recorder;
mod source;

pub use keys::user_facing_keys;
pub use source::{SettingOrigin, SettingProvenance, SettingSource};

pub(crate) use defaults::{apply_coding_defaults, apply_user_facing_defaults};
pub(crate) use recorder::{Recorder, record_endpoint_fields};

#[cfg(test)]
pub(crate) use defaults::USER_FACING;

#[cfg(test)]
#[path = "provenance_tests.rs"]
mod tests;
