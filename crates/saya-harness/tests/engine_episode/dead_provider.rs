//! A run step is bounded when its provider never answers.
//!
//! A dropped connection is a retryable transport failure, so a provider's
//! retry schedule is spent inside one receive call. The engine's bounded step
//! retry re-runs that schedule for each of its three attempts — with the
//! product's eight delays one dead provider costs ~287 s of wall clock. The
//! step's elapsed budget must reach the episode's control, not only the
//! engine's clock, so the loop never starts a retry the budget cannot finish.
//!
//! The scripted provider states a two-delay schedule of its own: the property
//! under test is the budget arithmetic, not the length of the product's
//! schedule, and a paused tokio clock is not usable here — the run's store
//! keeps timers of its own, and a paused clock jumps over them. The product's
//! default schedule is exercised end to end by `run_cli`.

use super::*;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

/// A provider that refuses every call as a transient transport failure — the
/// shape the loop's retry schedule exists for — and answers once the
/// connection has come back.
#[derive(Default)]
struct DeadProvider {
    calls: AtomicUsize,
    /// How many calls are refused before the connection comes back.
    answer_after: usize,
}

impl DeadProvider {
    /// A provider that never answers.
    fn dead() -> Self {
        Self {
            answer_after: usize::MAX,
            ..Self::default()
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ChatProvider for DeadProvider {
    fn name(&self) -> &str {
        "dead"
    }

    fn retry_delays(&self) -> Vec<Duration> {
        vec![Duration::from_millis(1); 2]
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call >= self.answer_after {
            return Ok(ChatResponse::new(ChatMessage::text("assistant", "back")));
        }
        Err(ProviderError::Unavailable(
            "the provider accepted and dropped the connection".into(),
        ))
    }
}

/// The one step of a plan whose budget states `wall_clock` — the elapsed
/// dimension a run's `[limits.run]` table layers onto each step.
fn dead_plan(ceiling: Duration) -> RunPlan {
    let mut budget = Budgets::default();
    budget.wall_clock = Some(ceiling);
    RunPlan::new(vec![step_with_budget(
        "read a value the provider never answers for",
        Some(budget),
    )])
    .unwrap()
}

/// Drives the plan's one step and reports what the step did.
async fn drive(dead: &DeadProvider, plan: &RunPlan) -> (Result<(), EpisodeError>, RunState) {
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        Arc::new(RecordingTools::default()),
        vec![def("probe", LocalStateEffect::None)],
    )];
    let run = approved_run("dead-provider").await;
    let (sink, driver) = driver_and_sink(&run, dead, &approval, &toolsets, false);
    let workspace = run.workspace();
    let result = driver.run_step(&sink, plan, 0, &workspace).await;
    let state = sink.state();
    let _ = fs::remove_dir_all(run.root);
    (result, state)
}

/// A step whose elapsed budget cannot hold a retry ends at its first refused
/// call instead of replaying the whole provider schedule on every attempt.
#[tokio::test]
async fn a_step_against_a_dead_provider_ends_within_its_elapsed_budget() {
    // Shorter than the loop's retry reserve: no retry can fit inside it.
    let plan = dead_plan(Duration::from_secs(2));
    let dead = DeadProvider::dead();
    let (result, state) = drive(&dead, &plan).await;

    assert_eq!(
        dead.calls(),
        1,
        "one call inside the budget, never the schedule replayed per attempt"
    );
    assert!(
        matches!(
            result,
            Err(EpisodeError::BudgetStopped {
                step: 0,
                reason: PauseReason::WallClockExceeded,
            })
        ),
        "the step's own elapsed budget is the pause cause: {result:?}"
    );
    assert_eq!(
        state,
        RunState::Paused,
        "a spent step budget pauses the run"
    );
}

/// A budget with room for retries still spends them: the step's clock bounds
/// the schedule, it does not disable it.
#[tokio::test]
async fn a_budget_that_holds_a_retry_still_retries() {
    let dead = DeadProvider {
        answer_after: 2,
        ..DeadProvider::default()
    };
    let plan = dead_plan(Duration::from_secs(60));
    let (result, state) = drive(&dead, &plan).await;

    assert_eq!(
        dead.calls(),
        3,
        "two retries inside the budget, then the answer"
    );
    assert!(result.is_ok(), "the step completes: {result:?}");
    assert_eq!(state, RunState::Completed);
}
