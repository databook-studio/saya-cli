//! The operator a chat session's calls are put to, whoever asks them: the root's
//! own tool, a helper's, or a background job's. Every ask becomes one
//! [`Question`] — the caller's line, the call's facts, the grant a session-wide
//! yes would record — and is delivered through the one front the session drives,
//! so the surface states every caller's call the same way.
//!
//! **One pending question.** A question is delivered only while no other one is
//! open. An ask that arrives meanwhile waits its turn — in the order the asks
//! arrived — and is delivered the moment the open one is answered; it is never
//! refused for being early.
//!
//! **The answer.** A yes is reported as the caller's own `Answer`; a no is a
//! normal denial. Only a session-wide yes is recorded in this run's store (see
//! [`Operator::grants`]), under the grant the question named: one call's yes
//! leaves the store alone, so the next identical call is asked again.
//!
//! **The wait.** A person is answering: no deadline of this operator's ends the
//! wait, only the answer, or the turn's stop. The turn asking is stated with
//! [`SessionOperator::set_turn`]: its stop denies every pending and waiting ask,
//! because a cancelled turn's calls must not run on it.
//!
//! The facts are model-authored, so they are sanitised before they reach the
//! front's screen, and a call naming no facts this front could show in full is
//! refused outright — the operator never answers for text it cannot read.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use saya_agent::{CancellationToken, SessionGrants};
use serde_json::Value;

use super::{ChatFront, Question};
use crate::agent::tools::consent::{Answer, Operator};
use crate::agent::tools::shell_policy::sanitise_prompt_text;

/// The longest command, working directory, path or URL one question shows, in
/// characters. A longer one is refused before anybody is asked: the cut tail of
/// a command could carry anything, and the operator would be approving text it
/// never saw.
const MAX_FACTS_CHARS: usize = 2048;

/// The operator of one chat run: the front's question slot, the turn that is
/// asking, and the run's "yes for this run" store.
pub(crate) struct SessionOperator {
    front: Arc<dyn ChatFront>,
    /// The turn currently asking. A stop of it denies every ask that turn left
    /// pending or waiting; the operator outlives one turn, so this is stated
    /// before each turn's asks.
    turn: Mutex<CancellationToken>,
    /// This run's store: the grants its operator has already answered a
    /// session-wide yes for. Shared by the root and every helper of the run
    /// through [`Operator::grants`], and never persisted — it ends with the run.
    grants: SessionGrants,
    /// The session's own number for the question being built, so one question
    /// can be named without naming a caller.
    next_id: AtomicU64,
    /// The one pending question's place: an ask holds it from the question it
    /// is about to put to the answer that came back, and a later ask waits here
    /// rather than being refused.
    queue: tokio::sync::Mutex<()>,
}

impl SessionOperator {
    pub(crate) fn new(front: Arc<dyn ChatFront>) -> Self {
        Self {
            front,
            turn: Mutex::new(CancellationToken::new()),
            grants: SessionGrants::default(),
            next_id: AtomicU64::new(0),
            queue: tokio::sync::Mutex::new(()),
        }
    }

    /// The turn now asking. Its stop drops that turn's pending and waiting asks
    /// with [`Answer::Deny`]; a later turn states its own token here.
    pub(crate) fn set_turn(&self, turn: CancellationToken) {
        *self.turn.lock().unwrap_or_else(PoisonError::into_inner) = turn;
    }

    /// The turn now asking, read once per ask so no lock is held across a wait.
    fn current_turn(&self) -> CancellationToken {
        self.turn
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The question one call becomes, or `None` for a call whose facts cannot
    /// be shown in full — a refusal, never an empty question.
    fn question(&self, caller: &str, arguments: &Value) -> Option<Question> {
        let facts = facts(arguments)?;
        Some(Question {
            id: self.next_id.fetch_add(1, Ordering::Relaxed) + 1,
            caller: caller.to_owned(),
            facts,
            grant: crate::grant_token::shell_token_for_host_route(arguments),
            // The change the call makes, from its own bytes: the panel paints
            // it under the path, so the operator answers for what they read.
            change: crate::change_diff::of(arguments)
                .map(|change| change.rows)
                .unwrap_or_default(),
        })
    }
}

#[async_trait]
impl Operator for SessionOperator {
    /// The run's own store, shared by every caller of the run through the
    /// prompt it is composed into.
    fn grants(&self) -> Option<SessionGrants> {
        Some(self.grants.clone())
    }

    /// One call, admitted on either yes: a session-wide one is recorded by
    /// [`Operator::answer`], so the two are not lost on the way back.
    async fn ask(&self, caller: &str, arguments: &Value) -> bool {
        self.answer(caller, arguments).await != Answer::Deny
    }

    async fn answer(&self, caller: &str, arguments: &Value) -> Answer {
        let Some(question) = self.question(caller, arguments) else {
            return Answer::Deny;
        };
        let turn = self.current_turn();
        // Waiting its turn is a wait of this ask's own: a stopped turn ends it
        // here, before the front is ever reached.
        let _pending = tokio::select! {
            _ = turn.cancelled() => return Answer::Deny,
            pending = self.queue.lock() => pending,
        };
        // The turn may have stopped while this ask waited its turn.
        if turn.is_cancelled() {
            return Answer::Deny;
        }
        let answer = tokio::select! {
            _ = turn.cancelled() => Answer::Deny,
            answer = self.front.ask(&question) => answer,
        };
        if answer == Answer::Session
            && let Some(token) = question.grant.as_deref()
        {
            self.grants.grant(token);
        }
        answer
    }
}

/// The facts body the operator reads before answering: the call's own target —
/// a shell's command and working directory, a query and the connection it runs
/// on, a plan, a path, or a URL — sanitised and bounded, in the shapes every
/// approval prompt of this crate states. `None` for a call that names no target
/// this operator can show in full: refusing is the safe answer, and the model is
/// free to send a shorter one.
fn facts(arguments: &Value) -> Option<String> {
    if let Some(command) = arguments.get("command").and_then(Value::as_str) {
        let workdir = arguments
            .get("workdir")
            .and_then(Value::as_str)
            .unwrap_or(".");
        if command.chars().count() > MAX_FACTS_CHARS || workdir.chars().count() > MAX_FACTS_CHARS {
            return None;
        }
        return Some(format!(
            "in: {}\n$ {}",
            sanitise_prompt_text(workdir),
            sanitise_prompt_text(command)
        ));
    }
    if let Some(sql) = arguments.get("sql").and_then(Value::as_str) {
        // The SQL safety layer bounds a statement well below this, so the check
        // is a shape guard rather than a bound the operator meets.
        if sql.chars().count() > MAX_FACTS_CHARS {
            return None;
        }
        // A call that names no connection runs on the primary one, and the
        // question says which database it is the primary of: that is the name
        // the answer's grant is recorded under.
        let connection = arguments
            .get("connection")
            .and_then(Value::as_str)
            .unwrap_or("the primary connection");
        if connection.chars().count() > MAX_FACTS_CHARS {
            return None;
        }
        return Some(format!(
            "on: {}\n{}",
            sanitise_prompt_text(connection),
            sanitise_prompt_text(sql)
        ));
    }
    if let Some(plan) = arguments.get("plan").and_then(Value::as_str) {
        // The plan gate caps a plan at the same number of characters this
        // question can show, so the check is a shape guard rather than a bound
        // the operator ever meets.
        if plan.chars().count() > MAX_FACTS_CHARS {
            return None;
        }
        return Some(sanitise_prompt_text(plan));
    }
    if let Some(path) = arguments.get("path").and_then(Value::as_str) {
        // The workspace tools bound a path well below this, so the check is a
        // shape guard here too.
        if path.chars().count() > MAX_FACTS_CHARS {
            return None;
        }
        return Some(format!("path: {}", sanitise_prompt_text(path)));
    }
    let url = arguments.get("url").and_then(Value::as_str)?;
    if url.chars().count() > MAX_FACTS_CHARS {
        return None;
    }
    Some(format!("GET {}", sanitise_prompt_text(url)))
}

#[cfg(test)]
#[path = "operator_answer_tests.rs"]
mod answer_tests;
#[cfg(test)]
#[path = "operator_pending_tests.rs"]
mod pending_tests;
