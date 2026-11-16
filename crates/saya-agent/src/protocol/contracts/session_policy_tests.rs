//! The engine's decisions, cell by cell: the mode match every approval
//! frontend carries, and the grant seam that pre-answers an ask.
//! Nothing grants a token yet, so the grant tests drive the store directly —
//! the seam must work before any frontend asks its question.

use super::approval::{AgentMode, WORKSPACE_WRITE_TOKEN};
use super::session_policy::{
    ApprovalChoice, ApprovalDecision, HEADLESS_ASK_REASON, PlanGateCell, SessionPolicy,
};
use super::{LocalStateEffect, ToolEffect};
use crate::protocol::approval::ApprovalPolicy;
use std::str::FromStr;

include!("session_policy_tests/approval_policy.rs");
include!("session_policy_tests/write_plan_policy.rs");
include!("session_policy_tests/accept_edits.rs");
include!("session_policy_tests/mode_switch.rs");
include!("session_policy_tests/frozen_mode.rs");
include!("session_policy_tests/plan_gate.rs");
