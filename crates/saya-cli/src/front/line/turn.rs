//! One line's turn, read off the stream its front renders.
//!
//! The chat turn reports everything on that one stream, so the session's
//! bookkeeping reads it there: the usage every attempt reported — a failed
//! attempt's consumed receipt and a retried round's included, which no settled
//! output could carry — the post-turn extraction call's usage, and the answer
//! the reader saw. Folding that into the session's accumulator is what the old
//! turn body's `take_answering_usage` settle did: an absent report is not a
//! reported zero, and a cancelled turn still spent what it consumed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentMetric, AnsweringObservation, TokenUsage, UsageCall,
};

use crate::answering_receipts::{AnsweringReceipts, add_usage};
use crate::chat::ChatOutcome;
use crate::interactive::SessionUsage;

/// One turn's stream, read as the session's bookkeeping before it is rendered.
#[derive(Default)]
pub(crate) struct TurnStream {
    usage: TurnUsage,
    /// The answer the turn said, as the reader saw it streamed.
    said: Mutex<String>,
    /// Whether the turn asked the operator a question: the turn that follows is
    /// that question's answer, so it is the same question and not the next one.
    asked: AtomicBool,
}

impl TurnStream {
    fn record(&self, event: &AgentEvent) {
        self.usage.record(event);
        match event {
            AgentEvent::AssistantText { text } => self.said().push_str(text),
            // A question the turn put to the reader: the next line is its
            // answer, and the question the turn was asked is still open.
            AgentEvent::ClarificationNeeded { .. } => self.asked.store(true, Ordering::SeqCst),
            // A retried attempt's answer replaces the partial one it discards,
            // so the turn's words are rolled back with it.
            AgentEvent::TurnReset => self.said().clear(),
            _ => {}
        }
    }

    /// Whether the turn that just settled asked the operator a question, taken
    /// once: the next turn continues that question rather than starting one.
    pub(crate) fn take_asked(&self) -> bool {
        self.asked.swap(false, Ordering::SeqCst)
    }

    /// What the turn said and spent, taken once at its settlement.
    pub(crate) fn take(&self) -> TurnRecord {
        TurnRecord {
            totals: self.usage.take(),
            answer: std::mem::take(&mut *self.said()),
        }
    }

    fn said(&self) -> MutexGuard<'_, String> {
        self.said.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One turn's usage, as its own stream reported it.
#[derive(Default)]
struct TurnUsage {
    /// The answering rounds: `Answer` usage and a failed attempt's consumed
    /// receipt, summed the way the terminal sink summed them.
    answering: AnsweringReceipts,
    /// The extraction call's usage; `None` when the provider reported none.
    learning: Mutex<Option<TokenUsage>>,
    /// The last answering report itself, not the sum of them: the last request
    /// is what the model's window was measured against, and a meter over the
    /// sum of a turn's rounds would overstate it.
    latest: Mutex<Option<TokenUsage>>,
    /// The costs the turn's calls reported, summed: what the provider priced
    /// this turn at, or `None` when no call reported a cost.
    cost: Mutex<Option<u64>>,
}

impl TurnUsage {
    fn record(&self, event: &AgentEvent) {
        self.answering.record(event);
        if let AgentEvent::Usage {
            call: UsageCall::Answer,
            usage,
        }
        | AgentEvent::FailedAttemptUsage { usage } = event
        {
            *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(*usage);
            if let Some(cost) = usage.cost_micros {
                let mut total = self.cost.lock().unwrap_or_else(PoisonError::into_inner);
                *total = Some(total.unwrap_or(0).saturating_add(cost));
            }
        }
        if let AgentEvent::Usage {
            call: UsageCall::Extraction,
            usage,
        } = event
        {
            let mut learning = self.learning.lock().unwrap_or_else(PoisonError::into_inner);
            add_usage(learning.get_or_insert_default(), *usage);
        }
    }

    fn take(&self) -> TurnTotals {
        TurnTotals {
            answering: self.answering.take(),
            learning: self
                .learning
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
            latest: self
                .latest
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
            cost_micros: self
                .cost
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        }
    }
}

/// What one settled turn hands the session: how it ended, what it said, and
/// what it spent.
pub(crate) struct SettledTurn {
    pub(crate) outcome: ChatOutcome,
    pub(crate) record: TurnRecord,
}

/// One turn's words and totals, taken at its settlement.
pub(crate) struct TurnRecord {
    pub(crate) totals: TurnTotals,
    pub(crate) answer: String,
}

/// One turn's totals, each absent when the provider reported nothing.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct TurnTotals {
    pub(crate) answering: Option<TokenUsage>,
    pub(crate) learning: Option<TokenUsage>,
    /// The turn's last answering report as it arrived: the request the model's
    /// window was measured against, which the summed `answering` cannot state.
    pub(crate) latest: Option<TokenUsage>,
    /// What the turn's calls reported they cost, summed; `None` when none did.
    pub(crate) cost_micros: Option<u64>,
}

impl TurnTotals {
    /// Folds this turn into the session's accumulator as today's settle did: an
    /// answered turn adds its answering usage only when the provider reported
    /// any (a silent provider adds nothing), while a failed or cancelled turn
    /// counts the call it consumed even at zero — absent is not a reported
    /// zero. The extraction call is recorded only when it reported.
    pub(crate) fn fold_into(&self, session: &mut SessionUsage, outcome: &ChatOutcome) {
        // Every settled turn is one request this session made, even when the
        // provider completed it without reporting usage: the count is separate
        // from the reported-usage turns, so a silent provider is not "no
        // requests yet".
        session.count_request();
        match outcome {
            ChatOutcome::Failed { .. } | ChatOutcome::Cancelled => {
                if let Some(usage) = &self.answering {
                    session.answering.record_call(usage);
                }
            }
            ChatOutcome::Answered | ChatOutcome::Stopped(_) | ChatOutcome::Paused { .. } => {
                if let Some(usage) = &self.answering {
                    session.record(usage);
                }
            }
        }
        session.record_learning(self.learning);
    }
}

/// The turn's stream, read for the session before the front renders it: events
/// and metrics pass through unchanged and in order.
pub(crate) struct StreamSink {
    inner: Arc<dyn AgentEventSink>,
    stream: Arc<TurnStream>,
}

impl StreamSink {
    pub(crate) fn new(inner: Arc<dyn AgentEventSink>, stream: Arc<TurnStream>) -> Self {
        Self { inner, stream }
    }
}

#[async_trait]
impl AgentEventSink for StreamSink {
    async fn emit(&self, event: AgentEvent) {
        self.stream.record(&event);
        self.inner.emit(event).await;
    }

    async fn observe_answering(&self, observation: AnsweringObservation) {
        self.inner.observe_answering(observation).await;
    }

    async fn metric(&self, metric: AgentMetric) {
        self.inner.metric(metric).await;
    }

    async fn background_exit(&self, text: &str) {
        self.inner.background_exit(text).await;
    }
}
