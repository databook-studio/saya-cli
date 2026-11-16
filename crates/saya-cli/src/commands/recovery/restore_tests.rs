//! Tests for `restore_change`: exact-byte restore through the guarded
//! workspace operations, guarded removal of created files, and every
//! refusal leaving the file byte-identical.

use std::fs;

use super::testing::*;
use super::*;
use saya_types::{CheckpointState, EditSource, RestoreEligibility};

include!("restore_tests/restore_success.rs");

include!("restore_tests/changed_file_refusals.rs");

include!("restore_tests/record_and_workspace_refusals.rs");

include!("restore_tests/symlink_and_cleanup.rs");
