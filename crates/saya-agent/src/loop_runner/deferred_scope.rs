//! A deferred answering scope: a turn's budget that counts nothing until armed.
//!
//! A database budget arms on a turn's *first database use* (`ref/05`, §2-§4),
//! not when the turn starts: a turn that never touches the database must not
//! spend a database request, tool call, known token, or second. This scope is
//! that shape. Created unarmed on a turn's token, it admits everything and
//! counts nothing; once [`DeferredAnsweringScope::arm`] hands it a
//! [`TurnPolicy`] it admits and refuses exactly as the supplied
//! [`AnsweringScope`](super::AnsweringScope) does with that policy.
//!
//! Two properties the rest of the run depends on:
//!
//! * **One clock.** The elapsed dimension is a deadline on the same
//!   [`tokio::time::Instant`] clock the request deadline
//!   (`loop_runner::receive_stream::RequestDeadline`) and the job's elapsed
//!   owner (`ProgressClassifier::remaining_wall`) already read. Arming stores
//!   an instant and every reading compares against it at the decision point;
//!   no timer task is spawned, so a turn waiting on this budget waits on the
//!   owner it already had (`ref/SPIKE-S0`, "Implications — C42").
//! * **One stop owner.** A budget stop latches the token this scope was created
//!   on — the turn's — and nothing else, so the run stays open (3.4a).

use std::sync::Arc;
use std::time::Duration;

use super::AnsweringLimits;
use super::admission::Admission;
use crate::{
    AgentError, AgentEventSink, BudgetDimension, CancellationToken, TokenUsage, TurnPolicy,
};
use tokio::sync::Mutex;

/// What one deferred scope has counted, and how long it has been armed for.
///
/// Every field is zero (or absent) before `arm`: an unarmed scope takes no
/// measurement at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeferredCounts {
    pub requests: u64,
    pub tool_calls: u64,
    /// `None` until a request reports usage — the same rule the supplied
    /// scope's reported-token counter keeps, which never invents a reported
    /// zero.
    pub known_tokens: Option<u64>,
    /// Time since the arm instant; zero while unarmed.
    pub elapsed: Duration,
}

/// An answering scope created unarmed: it counts nothing until armed.
#[derive(Clone)]
pub struct DeferredAnsweringScope {
    /// `None` — unarmed: nothing counted, nothing refused.
    state: Arc<Mutex<Option<Armed>>>,
    /// The turn's own stop signal, the only token a budget stop latches.
    turn: CancellationToken,
}

/// The counting owner, the arm instant, and the armed elapsed deadline.
struct Armed {
    /// The counter and refusal rules the supplied scope uses, with the merged
    /// policy as its ceilings.
    admission: Admission,
    /// The instant this turn's budget began. Every elapsed reading is taken
    /// from here, on the tokio clock.
    armed_at: tokio::time::Instant,
    /// `armed_at + max_elapsed`, when the policy states an elapsed ceiling.
    expires_at: Option<tokio::time::Instant>,
}

impl Armed {
    /// Refuses on the elapsed dimension once its deadline has passed. The
    /// reading is the same clock the deadline was set on, so paused time and
    /// the job's own elapsed owner agree.
    fn check_elapsed(&self, turn: &CancellationToken) -> Result<(), AgentError> {
        if self.remaining().is_some_and(|left| left.is_zero()) {
            return Err(self
                .admission
                .stop_preflight(turn, BudgetDimension::ElapsedTime));
        }
        Ok(())
    }

    /// What the elapsed ceiling has left, or `None` when none is armed.
    fn remaining(&self) -> Option<Duration> {
        self.expires_at
            .map(|at| at.saturating_duration_since(tokio::time::Instant::now()))
    }

    /// How long this scope has been armed.
    fn elapsed(&self) -> Duration {
        tokio::time::Instant::now().saturating_duration_since(self.armed_at)
    }
}

impl DeferredAnsweringScope {
    /// Creates one deferred scope on a turn's token. Nothing is armed and
    /// nothing is counted.
    pub fn new(turn: CancellationToken) -> Self {
        Self {
            state: Arc::new(Mutex::new(None)),
            turn,
        }
    }

    /// The turn token this scope's budget stop latches.
    pub fn turn_token(&self) -> CancellationToken {
        self.turn.clone()
    }

    /// Whether a policy has been armed yet.
    pub async fn is_armed(&self) -> bool {
        self.state.lock().await.is_some()
    }

    /// Arms this turn's budget on the answering engine's own counter and
    /// refusal rules. Arming again keeps the tightest ceiling per dimension,
    /// and the elapsed clock always starts at this instant, never at the
    /// turn's start.
    pub async fn arm(&self, policy: TurnPolicy) {
        let now = tokio::time::Instant::now();
        let mut state = self.state.lock().await;
        let armed = state.get_or_insert_with(|| Armed {
            admission: Admission::new(AnsweringLimits::default(), true),
            armed_at: now,
            expires_at: None,
        });
        armed.admission.limits = TurnPolicy::from_answering(armed.admission.limits, None)
            .tightest(policy)
            .answering_limits();
        if let Some(limit) = policy.max_elapsed {
            // An elapsed ceiling is a deadline, so the tighter of two arms is
            // the earlier instant: two arms at different moments compare as
            // deadlines, not as durations.
            let deadline = now + limit;
            armed.expires_at = Some(armed.expires_at.map_or(deadline, |held| held.min(deadline)));
        }
    }

    /// Admits one logical provider request, or refuses it once the armed policy
    /// is spent. An unarmed scope admits every request and counts none.
    pub async fn admit_request(&self, sink: &dyn AgentEventSink) -> Result<(), AgentError> {
        let mut state = self.state.lock().await;
        let Some(armed) = state.as_mut() else {
            return Ok(());
        };
        armed.check_elapsed(&self.turn)?;
        armed.admission.request(sink, &self.turn).await
    }

    /// Charges `count` requested tool calls and refuses the batch once the
    /// armed ceiling is passed — the supplied scope's order: observe, then
    /// enforce.
    pub async fn admit_tool_batch(
        &self,
        count: u64,
        sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        let mut state = self.state.lock().await;
        let Some(armed) = state.as_mut() else {
            return Ok(());
        };
        armed.check_elapsed(&self.turn)?;
        armed
            .admission
            .observe_tools(count, sink, &self.turn)
            .await?;
        armed.admission.enforce_tools(&self.turn)
    }

    /// Adds what one settled request reported and refuses once the armed
    /// known-token ceiling is reached.
    pub async fn settle_tokens(&self, usage: Option<TokenUsage>) -> Result<(), AgentError> {
        let mut state = self.state.lock().await;
        let Some(armed) = state.as_mut() else {
            return Ok(());
        };
        armed.check_elapsed(&self.turn)?;
        armed.admission.settle(usage, &self.turn)
    }

    /// What the armed elapsed ceiling has left, on the clock the request
    /// deadline and the job's elapsed owner read. `None` while unarmed, and
    /// when the armed policy states no elapsed ceiling.
    pub async fn remaining_elapsed(&self) -> Option<Duration> {
        self.state.lock().await.as_ref().and_then(Armed::remaining)
    }

    /// What this scope has counted so far. An unarmed scope counts nothing.
    pub async fn counts(&self) -> DeferredCounts {
        let state = self.state.lock().await;
        state
            .as_ref()
            .map_or_else(DeferredCounts::default, |armed| DeferredCounts {
                requests: armed.admission.requests,
                tool_calls: armed.admission.tool_calls,
                known_tokens: armed.admission.reported_tokens,
                elapsed: armed.elapsed(),
            })
    }
}
