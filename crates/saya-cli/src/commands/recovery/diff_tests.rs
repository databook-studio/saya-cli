//! Tests for the bounded diff: exact unified text, the created/missing
//! labels, every size/line/cell/output bound, and pathological shapes
//! that must stay fast and bounded.

use std::fs;
use std::time::Instant;

use super::diff::{DIFF_MAX_BYTES, DIFF_MAX_LINES, DIFF_MAX_OUTPUT_LINES};
use super::testing::*;
use super::*;
use saya_types::EditSource;

include!("diff_tests/unified_rendering.rs");

include!("diff_tests/bounds.rs");

include!("diff_tests/pathological_shapes.rs");

include!("diff_tests/change_metadata_and_refusals.rs");
