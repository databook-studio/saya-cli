//! Tests for rendering coding command results, split by command concern.

use super::render_coding_result;
use crate::cli::CodingCommand;
use serde_json::json;

mod reconcile;
mod review;
mod views;
