use saya_types::PauseReason;

use super::EngineEventSink;
use crate::engine::{
    clock::{ElapsedClockError, system_time_ms},
    state::RunState,
    transitions::TransitionEvent,
};

impl EngineEventSink {
    fn observe_elapsed_clock(&self) -> Result<bool, ElapsedClockError> {
        let mut clock = self.elapsed_clock.lock().expect("engine sink clock lock");
        let Some(clock) = clock.as_mut() else {
            return Ok(false);
        };
        let changed = clock.observe(system_time_ms()?, (self.clock)())?;
        if changed {
            self.journal
                .append(&clock.event())
                .map_err(|_| ElapsedClockError::InvalidJournal)?;
        }
        Ok(clock.remaining().is_zero())
    }
}

impl EngineEventSink {
    /// The per-tick budget checks, armed while the run executes. Past any
    /// ceiling the tick pauses the run exactly like any other transition;
    /// emit has no error channel, so any failure the pause meets is held as
    /// a diagnostic rather than dropped.
    ///
    /// Tokens are checked after the fold, never before: the tokens an event
    /// reports were already spent, so the ceiling is a stop-after, not a
    /// stop-before. A run pauses the tick *after* it crosses — which is the
    /// honest reading of a ceiling nobody can enforce mid-request.
    ///
    /// The download latch is checked between the token ceiling and the wall
    /// clock. The latch is level-triggered, so the pause lands on the first
    /// tick after the trip — in practice the tripping download's own
    /// completion event, which the model has already seen as the typed
    /// `BudgetExhausted` tool error: the episode ends informed, the run
    /// stops at the step boundary, byte-for-byte the wall-clock posture.
    pub(super) async fn tick(&self) {
        if self.state() != RunState::Executing {
            return;
        }
        let clock_expired = self.observe_elapsed_clock().unwrap_or(true);
        let reason = if self.tokens_exhausted() || self.downloads_exhausted() {
            PauseReason::BudgetExhausted
        } else if clock_expired
            || self
                .deadline
                .is_some_and(|deadline| (self.clock)() >= deadline)
        {
            PauseReason::WallClockExceeded
        } else {
            return;
        };
        if let Err(error) = self.record(TransitionEvent::Pause(reason)).await {
            self.hold_diagnostic(error);
        }
    }

    /// Whether the run's shared download wallet has recorded a refusal — the
    /// latch a download trip sets on every refused claim. `None` (the run
    /// did not approve fetch) leaves the check inert, exactly like an unset
    /// token ceiling.
    fn downloads_exhausted(&self) -> bool {
        self.download_budget
            .as_ref()
            .is_some_and(|budget| budget.tripped())
    }

    /// Whether the run has spent its declared token ceiling. The sum is the
    /// run's whole spend: the tokens the durable record already carries (a
    /// resume seeds them into the totals) plus the calls this sink has seen.
    /// Input and output are summed because the budget is what the run costs,
    /// and a ceiling that counted only one half would be a ceiling on
    /// nothing in particular. Figures no call reported stay out of the sum
    /// rather than counting as zero.
    fn tokens_exhausted(&self) -> bool {
        let Some(ceiling) = self.token_ceiling else {
            return false;
        };
        let usage = self.usage();
        usage
            .carried_tokens
            .saturating_add(usage.input_tokens)
            .saturating_add(usage.output_tokens)
            >= ceiling
    }
}
