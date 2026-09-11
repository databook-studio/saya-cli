//! The run event contract: one serde-tagged [`RunEvent`] per NDJSON line.
//!
//! Every lifecycle transition, step boundary, and usage report the engine
//! emits is one of these events. They are appended to the run's
//! `events.ndjson` journal and carried on the headless wire, so each event
//! must serialize to exactly one line — the property the contract tests pin.

use serde::{Deserialize, Serialize};

use super::plan::Deliverable;

/// The run's event stream. Lifecycle variants follow the run state machine
/// (`planned → approved → executing ⇄ paused → completed | failed |
/// cancelled`); step variants carry the step's index in the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RunEvent {
    /// The run was created and claimed its directory.
    RunStarted,
    /// The plan, scopes, and budgets were approved (or pre-authorised
    /// headless). No implicit approval exists.
    PlanApproved,
    /// A step's episode began.
    StepStarted { step: usize },
    /// A step completed.
    StepCompleted { step: usize },
    /// A step's episode failed; the engine retries it bounded, then pauses.
    StepFailed { step: usize },
    /// A step's declared deliverables, resolved against the workspace at the
    /// step's completion: one entry per declared output, in declaration
    /// order. Recorded before the step's own completion — the manifest is
    /// part of what completing means — so a journal that ends here reads as
    /// a step still in flight.
    Deliverables {
        step: usize,
        entries: Vec<Deliverable>,
    },
    /// The run paused — resumable, never a silent stop. Carries the cause.
    Paused { reason: PauseReason },
    /// The run completed successfully.
    Completed,
    /// The run failed terminally, by cause.
    Failed { code: RunFailureCode },
    /// The user cancelled the run.
    Cancelled,
    /// Usage for one endpoint. Each count is carried only when it is known:
    /// an unreported count serializes `null` and means "unknown" — never
    /// zero — because a provider that reports nothing must not be read as
    /// having cost nothing.
    Usage {
        endpoint: String,
        tokens: Option<u64>,
        turns: Option<u64>,
        tool_calls: Option<u64>,
        /// The cache reads the provider reported for this call; `None` when
        /// it reported no figure — a provider that says nothing about its
        /// cache must not be read as having missed it. `#[serde(default)]`
        /// keeps journals written before the field existed parseable.
        #[serde(default)]
        cached_input_tokens: Option<u64>,
        /// The cache writes the provider reported for this call, same rule
        /// as above.
        #[serde(default)]
        cache_creation_input_tokens: Option<u64>,
    },
}

/// Why a run paused. A paused run is resumable at the first incomplete step;
/// the reason is what `saya run show` explains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PauseReason {
    /// A declared budget tripped.
    BudgetExhausted,
    /// The engine's wall-clock deadline passed.
    WallClockExceeded,
    /// A step kept failing past the engine's bounded retries.
    StepFailedAfterRetry,
    /// The state store became unavailable mid-run; the run pauses rather
    /// than weakening a gate to proceed.
    StoreUnavailable,
    /// The process holding the run died; recorded when the run is next
    /// picked up.
    ProcessDeath,
    /// The user paused the run.
    UserPaused,
}

/// The typed cause of a terminal run failure. The CLI maps these to the
/// documented exit codes; the code itself stays honest about the layer that
/// failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RunFailureCode {
    /// A statement or query failed the read-only safety gate.
    SafetyQuery,
    /// The provider or agent layer failed.
    Provider,
    /// A connection or configuration problem.
    ConnectionConfig,
}
