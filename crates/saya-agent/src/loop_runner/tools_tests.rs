//! The side-effect guard and the tool-message cap: the S2 loop seam's pins.
//!
//! Phase 7 packet 2: a failed tool's human-facing summary names *why* it
//! failed, so a safety-layer refusal reads differently from a runtime
//! failure. The model's JSON path is unchanged — the full error still
//! reaches it.

use super::super::tool_batch::turn_result_caps;
use super::super::tool_policy::{
    ApprovalState, ExecutionDecision, PolicyDenial, execution_decision, external_side_effect_gated,
};
use super::*;
use crate::{AgentLimits, LocalStateEffect, ToolDefinition, ToolEffect, ToolError, ToolExecutor};

include!("tools_tests/policy_gates.rs");
include!("tools_tests/error_summaries.rs");
include!("tools_tests/redaction_notice.rs");
include!("tools_tests/result_shaping.rs");
