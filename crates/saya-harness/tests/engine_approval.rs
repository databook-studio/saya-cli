//! The approval gate at `planned → approved` — contract tests (M1-10).
//!
//! Four guarantees:
//! 1. A planned run refuses to begin a step until the plan is approved, the
//!    approval goes through the engine's `TransitionEvent` path once, and a
//!    second approval is refused — approval is per plan, granted once.
//! 2. A plan asking for a scope the approved set does not hold is refused
//!    by the propose gate rather than silently binding it — the same
//!    approved set refuses it, naming the missing scope; only an explicit
//!    widening binds. The engine proposes once, before approval; there is
//!    no separate mid-run revision flow, so this gate is the only one.
//! 3. Approving the plan does not produce a per-tool-call prompt afterwards:
//!    the steps complete with a tool that requires approval, and the
//!    channel-mirroring decider never prompts.
//! 4. The needs-approval refusal names the missing scope — the composition
//!    root's message can say what was missing, not refuse generically.

use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    LocalStateEffect, ProviderError, ToolCall, ToolEffect, ToolExecutor, read_only_permits,
};
use saya_harness::engine::{
    EngineEventSink, EpisodeCollaborators, EpisodeDriver, EpisodeError, EpisodeRun, ManifestBounds,
    PlanDriver, PlanError, PlanRejection, PlanRequest, RunState, SinkBudgets, StepToolset,
    TransitionEvent, UsageTotals,
};
use saya_harness::journal::Journal;
use saya_harness::workspace::Workspace;
use saya_store::{NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStore, SqliteStateStore};
use saya_types::{Budgets, Capabilities, RunId, StepSpec};
use tokio::sync::oneshot;

#[path = "engine_approval/fixtures.rs"]
mod fixtures;

#[path = "engine_approval/plan_transitions.rs"]
mod plan_transitions;

#[path = "engine_approval/approval_scope.rs"]
mod approval_scope;
