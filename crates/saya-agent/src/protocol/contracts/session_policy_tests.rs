//! The engine's decisions, cell by cell: the mode match every approval
//! frontend used to carry, and the grant seam that pre-answers an ask.
//! Nothing grants a token yet, so the grant tests drive the store directly —
//! the seam must work before any frontend asks its question.

use super::approval::AgentMode;
use super::session_policy::{ApprovalChoice, ApprovalDecision, SessionPolicy};
use super::{LocalStateEffect, ToolEffect};
use crate::protocol::approval::ApprovalPolicy;
use std::str::FromStr;

include!("session_policy_tests/approval_policy.rs");
include!("session_policy_tests/write_plan_policy.rs");
include!("session_policy_tests/frozen_mode.rs");
