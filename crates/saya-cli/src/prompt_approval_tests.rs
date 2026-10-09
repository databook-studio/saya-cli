//! Terminal approval semantics: what each `ApprovalPolicy` approves through
//! `TerminalApproval` (the decider behind `saya ask` and non-interactive
//! runs). The TUI's `ChannelApproval` (`interactive/tui/agent.rs`) mirrors the
//! read-only rule. The session's hoisted policy rides `from_session`: grants
//! recorded through one turn's decider stay in force for the next.

use crate::agent::tools::DatabaseTools;
use crate::approval_facts::ApprovalFacts;
use crate::grant_token::{TurnPrimary, grant_token};
use crate::local_metrics::{LocalMetrics, PhaseCell, outcome_index, phase_index};
use crate::prompt_approval::{TerminalApproval, approval_prompt, terminal_choice};
use saya_agent::{
    AgentMetricOutcome, AgentMetricPhase, ApprovalChoice, ApprovalDecider, ApprovalDecision,
    ApprovalPolicy, CancellationToken, LocalStateEffect, SessionPolicy, ToolDefinition, ToolEffect,
};
use std::sync::Arc;

fn side_effecting_tool() -> ToolDefinition {
    ToolDefinition {
        name: "run_program".into(),
        description: "spawns a process outside the agent".into(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    }
}

/// The session's `workspace_write` definition, the canonical grantable shape.
fn workspace_write_tool() -> ToolDefinition {
    crate::interactive::session_definitions::workspace_write()
}

fn database_tools() -> Vec<ToolDefinition> {
    DatabaseTools::definitions(true, false, false, false, true)
}

/// A composition that carries the write-shaped family: a bound workspace
/// root, the one fact `workspace_write`'s suggestion gates on (U8).
fn workspace_rooted_facts() -> ApprovalFacts {
    ApprovalFacts {
        workspace_root: Some(std::path::PathBuf::from("/home/user/proj")),
        ..ApprovalFacts::default()
    }
}

#[path = "prompt_approval_tests/consent_grouping.rs"]
mod consent_grouping;
#[path = "prompt_approval_tests/frozen_run.rs"]
mod frozen_run;
#[path = "prompt_approval_tests/grant_lifecycle.rs"]
mod grant_lifecycle;
#[path = "prompt_approval_tests/metrics.rs"]
mod metrics;
#[path = "prompt_approval_tests/mode_semantics.rs"]
mod mode_semantics;
