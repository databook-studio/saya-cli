//! The line front: the surface a chat session drives when stdin is a pipe or
//! the plain loop's terminal.
//!
//! It is today's headless projection, wrapped around the one seam the chat
//! layer talks to ([`ChatFront`]):
//!
//! - **`sink`** is the terminal sink the headless turn always printed through,
//!   so text, JSON and NDJSON output — text deltas, tool steps, metrics races,
//!   the `⏸ question` block — stays byte-identical to the path it replaced.
//! - **`ask`** is the terminal approval prompt: the call's own facts, the
//!   answers line, one typed answer read from stdin ([`ask`]).
//! - **`emit`** says a settled turn in today's wording: the truthful partial
//!   sentence on stderr for a budget stop, the guard's own message for a pause,
//!   `Request cancelled.` for a cancel, and the failure's message for anything
//!   else. The turn's outcome is also kept, so the loop above can read what the
//!   turn settled on without the session telling anyone twice.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use saya_agent::{AgentEventSink, CancellationToken};

use super::ask;
use crate::agent::tools::consent::Answer;
use crate::budget_wording::budget_partial_message;
use crate::chat::{ChatEvent, ChatFront, ChatOutcome, Question};
use crate::interactive::turn_summary::TallyHandle;
use crate::local_metrics::LocalMetrics;
use crate::local_metrics_format::print_metrics_section;
use crate::render::{RenderFormat, TerminalEvent, render_event};
use crate::stream_render::TerminalSink;

/// What a cancelled turn says, in today's words.
const CANCELLED: &str = "Request cancelled.";

/// The state behind the front's one lock.
struct Shared {
    /// The turn now running, so a question it left pending ends with it.
    turn: CancellationToken,
    /// How the last turn that ended settled: `None` until one has.
    ended: Option<ChatOutcome>,
}

/// One line front over one session.
pub(crate) struct LineFront {
    format: RenderFormat,
    /// Whether this surface may read an answer from stdin at all: the plain
    /// loop's live-terminal fact. A piped run denies every question unasked.
    can_prompt: bool,
    /// The tally every turn's sink records its tool events into, read by the
    /// scripted run's one terminal result.
    tally: TallyHandle,
    /// The run's own counters, printed after every settled turn.
    metrics: Option<Arc<LocalMetrics>>,
    shared: Mutex<Shared>,
}

impl LineFront {
    pub(crate) fn new(format: RenderFormat, can_prompt: bool, tally: TallyHandle) -> Self {
        Self {
            format,
            can_prompt,
            tally,
            metrics: None,
            shared: Mutex::new(Shared {
                turn: CancellationToken::new(),
                ended: None,
            }),
        }
    }

    /// The same front recording the run's counters, so `--verbose` prints the
    /// section the headless turn always printed.
    pub(crate) fn with_metrics(mut self, metrics: Option<Arc<LocalMetrics>>) -> Self {
        self.metrics = metrics;
        self
    }

    /// The turn now running: a pending question ends when its turn does.
    pub(crate) fn set_turn(&self, turn: CancellationToken) {
        self.lock().turn = turn;
    }

    /// How the last settled turn ended, taken once — the loop above reads it
    /// to word its exit status and to settle the session record.
    pub(crate) fn take_outcome(&self) -> Option<ChatOutcome> {
        self.lock().ended.take()
    }

    /// The tool tally every turn's sink recorded into: the scripted run's one
    /// terminal result reports it.
    pub(crate) fn tally(&self) -> crate::interactive::turn_summary::ToolTally {
        self.tally.lock().expect("tool tally").clone()
    }

    /// Says one settled turn in today's wording.
    fn say(&self, outcome: &ChatOutcome) {
        print_metrics_section(&self.metrics);
        if let Some(event) = wording(outcome) {
            let rendered = render_event(&event, self.format);
            print!("{}", rendered.stdout);
            eprint!("{}", rendered.stderr);
        }
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The one line a settled turn says, as today's headless path said it.
///
/// An answered turn says nothing extra: its text already reached the sink. A
/// budget stop says the truthful partial sentence, a pause says the guard's own
/// message, a cancel and a failure say theirs.
pub(crate) fn wording(outcome: &ChatOutcome) -> Option<TerminalEvent> {
    match outcome {
        ChatOutcome::Answered => None,
        ChatOutcome::Stopped(budget) => Some(TerminalEvent::Diagnostic {
            message: budget_partial_message(budget.dimension),
        }),
        ChatOutcome::Paused { message } => Some(TerminalEvent::Diagnostic {
            message: message.clone(),
        }),
        ChatOutcome::Failed { message } => Some(TerminalEvent::Error {
            message: message.clone(),
        }),
        ChatOutcome::Cancelled => Some(TerminalEvent::Diagnostic {
            message: CANCELLED.to_owned(),
        }),
    }
}

#[async_trait]
impl ChatFront for LineFront {
    fn sink(&self) -> Arc<dyn AgentEventSink> {
        // A sink per turn: the terminal renderer's bracket state is one turn's.
        Arc::new(TerminalSink::new(self.format).with_tally(Arc::clone(&self.tally)))
    }

    async fn ask(&self, question: &Question) -> Answer {
        let turn = self.lock().turn.clone();
        ask::put(self.can_prompt, &turn, question).await
    }

    fn emit(&self, event: ChatEvent) {
        match event {
            ChatEvent::TurnEnded(outcome) => {
                self.say(&outcome);
                self.lock().ended = Some(outcome);
            }
            // The session's own words — a message replaced before it ran, a
            // prepare that failed. Said as an error line: it is a fact the
            // operator must read, and stderr is the loop's diagnostic channel.
            ChatEvent::Notice { message } => {
                let rendered = render_event(&TerminalEvent::Error { message }, self.format);
                print!("{}", rendered.stdout);
                eprint!("{}", rendered.stderr);
            }
            // The rest is the sink's or the operator's: a question is put
            // through `ask`, a capture is the front's preview, a ready event
            // names a run the plain loop already described at startup.
            ChatEvent::Ready { .. }
            | ChatEvent::Question(_)
            | ChatEvent::QueryCaptured(_)
            | ChatEvent::Agent(_) => {}
        }
    }
}
