//! Resume — contract tests (M1-5, resume slice).
//!
//! Seven guarantees, one per test:
//! 1. A crash at **every** journal position: for each prefix of a driven
//!    two-step run's journal, resume continues at the first incomplete
//!    step and lands the run in the right final state — completed steps
//!    are not re-run and their events are not re-journalled.
//! 2. A completed run resumes to "nothing to do" rather than re-running
//!    the last step.
//! 3. A step executing at the crash restarts from its beginning, and the
//!    restart is visible in the journal.
//! 4. A second engine on the same run directory is refused while the
//!    first holds the lock.
//! 5. A journal with a torn final line — the crash that matters most —
//!    replays to the last whole event rather than erroring the run.
//! 6. The token ceiling binds the **run**, not each invocation: a resumed
//!    run seeded from the journal's usage record pauses at its cumulative
//!    spend, never at a re-armed ceiling.
//! 7. Seeding rides the repaired record: a torn usage line never counts
//!    toward the carried spend.
//! 8. The download budget binds the **run** the same way: a resumed run's
//!    wallet is seeded from the download spend the journal records, so a
//!    resumed download is refused what the record already holds the run
//!    having spent, and the resumed claims record the level they reached.

use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, CancellationReason, CancellationToken, ChatMessage, ChatProvider, ChatRequest,
    ChatResponse, LocalStateEffect, ProviderError, TokenUsage, ToolCall, ToolDefinition,
    ToolEffect, ToolError, ToolExecutionContext, ToolExecutor,
};
use saya_harness::HarnessError;
use saya_harness::engine::{
    EngineEventSink, EpisodeCollaborators, EpisodeDriver, EpisodeRequest, EpisodeRun,
    ManifestBounds, ResumeError, ResumeOutcome, ResumeRun, RunState, SinkBudgets, StepToolset,
    TransitionEvent, UsageTotals, resume,
};
use saya_harness::fetch::DownloadBudget;
use saya_harness::journal::Journal;
use saya_harness::lock::RunLock;
use saya_harness::workspace::Workspace;
use saya_store::{
    NewRun, RunBudgets, RunCapabilityFlags, RunRecord, RunStatus, RunStepRecord, RunStepStatus,
    RunStore, RunSummary, RunUsage, SqliteStateStore, StoreError,
};
use saya_types::{Capabilities, PauseReason, RunEvent, RunId, RunPlan, StepSpec};
use tokio::sync::oneshot;

#[path = "engine_resume/counts.rs"]
mod counts;

include!("engine_resume/fixtures/providers.rs");

include!("engine_resume/fixtures/store_adapter.rs");

include!("engine_resume/fixtures/stubs.rs");

include!("engine_resume/fixtures/crashed_run.rs");

include!("engine_resume/fixtures/driven_run.rs");

#[path = "engine_resume/journal_prefixes.rs"]
mod journal_prefixes;

#[path = "engine_resume/lifecycle.rs"]
mod lifecycle;

/// One journaled usage event, shaped exactly as the engine journals a
/// provider call: the combined input-plus-output figure under the ceiling's
/// arithmetic, with every figure no call reported absent.
fn journaled_usage(tokens: u64) -> RunEvent {
    RunEvent::Usage {
        endpoint: "orchestrator".into(),
        tokens: Some(tokens),
        turns: None,
        tool_calls: None,
        cached_input_tokens: None,
        cache_creation_input_tokens: None,
    }
}

#[path = "engine_resume/journal_repair.rs"]
mod journal_repair;

/// The resume inputs for a run with a declared token ceiling and a provider
/// whose turn reports usage — the budget posture of the tests below; every
/// other input is the crashed run's own.
fn budgeted_inputs<'a>(
    run: &'a CrashedRun,
    provider: &'a UsageProvider,
    ceiling: Option<u64>,
) -> ResumeRun<'a> {
    ResumeRun {
        run_id: run.run_id.clone(),
        store: run.store.clone(),
        plan: run.plan.clone(),
        incomplete_policy: saya_harness::engine::IncompletePolicy::Conservative,
        workspace: workspace(&run.root),
        collaborators: EpisodeCollaborators {
            provider,
            approval: &run.stubs.approval,
            toolsets: &run.toolsets,
            cancellation: CancellationToken::default(),
        },
        request: request(),
        bounds: bounds(),
        wall_clock: None,
        token_ceiling: ceiling,
        download_budget: None,
        journal_wire: None,
        agent_stream: None,
    }
}

#[path = "engine_resume/token_budget.rs"]
mod token_budget;

#[path = "engine_resume/deadline.rs"]
mod deadline;

// --- the download budget's carry (the wallet, not the latch) ------------------

include!("engine_resume/fixtures/downloads.rs");

#[path = "engine_resume/download_budget.rs"]
mod download_budget;

#[path = "engine_resume/uncertainty.rs"]
mod uncertainty;

#[path = "engine_resume/clock.rs"]
mod clock;

#[path = "engine_resume/retry_counts.rs"]
mod retry_counts;
