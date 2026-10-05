//! Run resume: rebuild a run from its journal and continue it. The journal
//! is the durable authority — the store is only its mirror — and the run
//! directory's single-writer lock is taken before anything is read, so a
//! second engine on the same run is refused outright.

mod contract;
mod effects;
pub use contract::{IncompletePolicy, ResumeEffect, ResumeError, ResumeOutcome, ResumeRun};

use std::{path::Path, sync::Arc};

use saya_agent::AnsweringObservation;
use saya_types::{PauseReason, RunEvent};

use crate::{
    engine::{
        AnsweringCountError,
        clock::ElapsedClock,
        episode::{EpisodeDriver, EpisodeRun},
        sink::{EngineEventSink, SinkBudgets},
        state::RunState,
        transitions::TransitionEvent,
        usage::{StepAnsweringCounts, UsageTotals},
    },
    fetch::DownloadBudget,
    journal::{Journal, JournalState, StepState, replay},
    lock::RunLock,
};
use effects::possible_effects;

/// Resumes the run in `run_dir` from its journal. The journal is replayed
/// into the state the run actually held when its process died, and the run
/// continues at the plan's first incomplete step through the same
/// [`TransitionEvent`] path a fresh run uses: resume never writes a state
/// directly, so a resumed run cannot reach a state the machine forbids.
///
/// An in-flight episode is deliberately **not** checkpointed. A step that
/// was executing when the process died restarts from its beginning, never
/// resumed mid-turn: a half-finished episode has already sent tool calls
/// whose effects we cannot replay or undo, so pretending to resume inside
/// one would be a lie about what happened. The restart is a fresh attempt
/// under the engine's bounded retry, visible in the journal as a second
/// `StepStarted` for the same step.
pub async fn resume(
    run_dir: impl AsRef<Path>,
    resumed: ResumeRun<'_>,
) -> Result<ResumeOutcome, ResumeError> {
    // Single writer first: a live holder means another engine owns the run.
    let _lock = RunLock::acquire(run_dir.as_ref().join("lock"))
        .map_err(|source| ResumeError::Lock { source })?;
    let journal = match &resumed.journal_wire {
        Some(wire) => Journal::open(run_dir.as_ref()).with_wire(Arc::clone(wire)),
        None => Journal::open(run_dir.as_ref()),
    };
    journal
        .truncate_torn_tail()
        .map_err(|source| ResumeError::Journal { source })?;
    // The repaired record, read once: the replay is the resume's authority
    // for where the run stood, and the same events seed the sink's usage
    // totals and the download wallet, so the token ceiling and the download
    // budget both measure the run's whole spend across invocations. Seeding
    // rides the repaired view — the read happens after the torn tail was
    // dropped — so a half-written line never counts toward the carried
    // spend, whichever budget it names.
    let events = journal
        .read()
        .map_err(|source| ResumeError::Journal { source })?;
    // The download wallet the composition root armed is shared by clone
    // with the fetch-capable steps' executors, so arming it here — before
    // the sink takes over and before any step runs — arms every holder: the
    // resumed run continues against the spend its journal already records
    // instead of a fresh wallet, the same carry the usage seeding gives the
    // token ceiling. The carry rides the repaired record — the read after
    // the torn tail was dropped — so a half-written level never counts, and
    // it happens before the sink is built, whose journaled-download
    // baseline is the wallet's level at construction.
    if let Some(budget) = &resumed.download_budget {
        budget.carry(DownloadBudget::carried_from_journal(&events));
    }
    let state = replay(&events);
    let initial = match position(&state) {
        Position::NoRun => return Ok(ResumeOutcome::NoRun),
        Position::Unapproved => return Ok(ResumeOutcome::Unapproved),
        Position::Terminal(state) => return Ok(ResumeOutcome::Settled { state }),
        Position::Approved => RunState::Approved,
        Position::Paused => RunState::Paused,
        Position::MidFlight => RunState::Executing,
    };
    let first_step = (0..resumed.plan.steps.len())
        .find(|&step| state.steps.get(&step) != Some(&StepState::Completed));
    let uncertain = first_step.and_then(|first_step| {
        if resumed.incomplete_policy == IncompletePolicy::Retry {
            return None;
        }
        (first_step..resumed.plan.steps.len()).find_map(|step| {
            let recorded_incomplete = matches!(
                state.steps.get(&step),
                Some(StepState::Started | StepState::Failed)
            );
            if !recorded_incomplete {
                return None;
            }
            let spec = &resumed.plan.steps[step];
            let effects = possible_effects(spec, resumed.collaborators.toolsets.get(step));
            (!effects.is_empty()).then(|| (step, spec.goal.clone(), effects))
        })
    });
    let replayed_counts = first_step.map(|step| StepAnsweringCounts::from_journal(&events, step));
    let elapsed = resumed
        .wall_clock
        .filter(|_| first_step.is_some() && uncertain.is_none())
        .map(|ceiling| ElapsedClock::resume_now(&events, ceiling));
    let carried_wall_clock = match &elapsed {
        Some(Ok(clock)) => Some(clock.remaining()),
        Some(Err(_)) => None,
        None => None,
    };
    let carried_counts = replayed_counts
        .as_ref()
        .and_then(|counts| counts.as_ref().ok())
        .copied()
        .unwrap_or_default();
    let sink = EngineEventSink::new(
        resumed.run_id.clone(),
        initial,
        journal.clone(),
        resumed.store.clone(),
        SinkBudgets {
            wall_clock: carried_wall_clock,
            token_ceiling: resumed.token_ceiling,
            download_budget: resumed.download_budget.clone(),
            carried_usage: UsageTotals::from_journal(&events),
        },
        std::time::Instant::now,
    )
    .with_answering_counts(carried_counts)
    .with_cancellation(resumed.collaborators.cancellation.clone());
    let mut sink = match &resumed.agent_stream {
        Some(stream) => sink.with_agent_stream(Arc::clone(stream)),
        None => sink,
    };
    if initial == RunState::Executing {
        // Record the death the journal could not. A refusal stays paused and
        // never records that execution resumed.
        sink.record(TransitionEvent::Pause(PauseReason::ProcessDeath))
            .await
            .map_err(|source| ResumeError::Transition { source })?;
    }
    if let Some((step, goal, effects)) = uncertain {
        return Ok(ResumeOutcome::IncompleteEffects {
            step,
            goal,
            effects,
        });
    }
    if let Some(Err(source)) = &elapsed {
        return Err(ResumeError::ElapsedClock { source: *source });
    }
    if let Some(step) = first_step
        && let Some(budget) = resumed.plan.steps[step].budget.as_ref()
        && (budget.turns.is_some() || budget.tool_calls.is_some())
    {
        let mut counts = match replayed_counts {
            Some(Ok(counts)) => counts,
            Some(Err(source)) => return Err(ResumeError::CountEvidence { step, source }),
            None => StepAnsweringCounts::default(),
        };
        if let Err(source) = counts.remaining(step, budget.turns, budget.tool_calls) {
            match source {
                AnsweringCountError::PendingBatch | AnsweringCountError::MissingInitialization
                    if resumed.incomplete_policy == IncompletePolicy::Retry => {}
                _ => return Err(ResumeError::CountEvidence { step, source }),
            }
            // Retry reconciles the crash's half-written evidence: a pending
            // request stays charged, its unobserved batch settling to zero —
            // the operator's accepted risk; missing initialization charged
            // nothing, so the fresh attempt restarts from fresh counts. The
            // settlement is journaled, not only counted in memory: the live
            // settle's closing row is what keeps a second crash after the
            // resumed attempt's own admission replaying as admit, batch,
            // admit instead of two admissions in a row, which
            // `from_journal` rejects under every policy.
            if source == AnsweringCountError::PendingBatch {
                journal
                    .append(&RunEvent::Usage {
                        endpoint: crate::endpoints::ORCHESTRATOR_ROLE.to_string(),
                        tokens: None,
                        turns: None,
                        tool_calls: Some(0),
                        cached_input_tokens: None,
                        cache_creation_input_tokens: None,
                    })
                    .map_err(|source| ResumeError::Journal { source })?;
                let batch = AnsweringObservation::ToolBatchObserved { requested: 0 };
                counts
                    .observe(batch)
                    .map_err(|source| ResumeError::CountEvidence { step, source })?;
            } else {
                counts = StepAnsweringCounts::default();
            }
            sink = sink.with_answering_counts(counts);
        }
    }
    let (sink, clock_exhausted) = match elapsed {
        Some(Ok(mut clock)) => {
            journal
                .append(&clock.event())
                .map_err(|source| ResumeError::Journal { source })?;
            clock.mark_persisted(std::time::Instant::now());
            let exhausted = clock.remaining().is_zero();
            (sink.with_elapsed_clock(clock), exhausted)
        }
        _ => (sink, false),
    };
    if matches!(initial, RunState::Executing | RunState::Paused) {
        sink.record(TransitionEvent::Resume)
            .await
            .map_err(|source| ResumeError::Transition { source })?;
    }
    if clock_exhausted && first_step.is_some() {
        if initial == RunState::Approved {
            sink.record(TransitionEvent::Begin)
                .await
                .map_err(|source| ResumeError::Transition { source })?;
        }
        sink.record(TransitionEvent::Pause(PauseReason::WallClockExceeded))
            .await
            .map_err(|source| ResumeError::Transition { source })?;
        let Some(first_step) = first_step else {
            unreachable!("clock enforcement is skipped after all steps complete");
        };
        return Ok(ResumeOutcome::Resumed {
            first_step,
            state: RunState::Paused,
        });
    }
    let Some(first_step) = first_step else {
        // Every step is complete but the completion was never recorded: the
        // journal proves the plan succeeded, so record it through the sink.
        sink.record(TransitionEvent::Complete)
            .await
            .map_err(|source| ResumeError::Transition { source })?;
        return Ok(ResumeOutcome::Settled {
            state: RunState::Completed,
        });
    };
    let driver = EpisodeDriver::new(
        resumed.collaborators,
        EpisodeRun {
            run_id: resumed.run_id.clone(),
            store: resumed.store.clone(),
            journal,
        },
        resumed.request,
        resumed.bounds,
    );
    for step in first_step..resumed.plan.steps.len() {
        if state.steps.get(&step) == Some(&StepState::Completed) {
            continue;
        }
        driver
            .run_step(&sink, &resumed.plan, step, &resumed.workspace)
            .await
            .map_err(|source| ResumeError::Episode { source })?;
    }
    Ok(ResumeOutcome::Resumed {
        first_step,
        state: sink.state(),
    })
}

/// Where the journal says the run stood when its process died.
enum Position {
    /// No `RunStarted` — the directory holds no run.
    NoRun,
    /// Started, never approved: nothing may run without approval.
    Unapproved,
    /// The journal's last lifecycle event is terminal.
    Terminal(RunState),
    /// Approved, never began — the first step begins fresh.
    Approved,
    /// Paused by a recorded reason; resume records the resume.
    Paused,
    /// A step was in flight: the process died executing.
    MidFlight,
}

fn position(state: &JournalState) -> Position {
    if !state.started {
        return Position::NoRun;
    }
    if !state.plan_approved {
        return Position::Unapproved;
    }
    match state.last {
        Some(RunEvent::Completed) => Position::Terminal(RunState::Completed),
        Some(RunEvent::Failed { .. }) => Position::Terminal(RunState::Failed),
        Some(RunEvent::Cancelled) => Position::Terminal(RunState::Cancelled),
        Some(RunEvent::Paused { .. }) => Position::Paused,
        // A step event with no terminal or pause after it — the process
        // died mid-flight, and its last step restarts from its beginning.
        _ if !state.steps.is_empty() => Position::MidFlight,
        _ => Position::Approved,
    }
}
