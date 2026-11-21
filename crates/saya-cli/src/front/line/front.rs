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
//!
//! Two things the old turn body did per turn, this front does now: the sink it
//! hands each turn is also read for the session's bookkeeping ([`super::turn`]
//! — the turn's usage and the words it said), and Ctrl-C is the turn's own stop
//! ([`super::signals`]) — the next line is a fresh turn of the same run.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use saya_agent::{AgentEventSink, CancellationToken};

use super::ask;
use super::signals::Signals;
use super::turn::{StreamSink, TurnRecord, TurnStream};
use crate::agent::question_scope::QuestionScope;
use crate::agent::tools::consent::Answer;
use crate::budget_wording::budget_partial_message;
use crate::chat::{ChatEvent, ChatFront, ChatOutcome, Question};
use crate::interactive::turn_summary::TallyHandle;
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

/// Where one turn's stream goes: the terminal's own renderer, or the sink the
/// surface above brought with it.
#[derive(Clone)]
enum Surface {
    /// The terminal: one renderer for this format, a fresh sink per turn.
    Terminal(RenderFormat),
    /// A sink the surface brought: the one-shot `ask` prints its answer through
    /// the very sink the old turn body printed on, so text and JSON stay the
    /// output they were.
    Given(Arc<dyn AgentEventSink>),
}

/// One line front over one session.
pub(crate) struct LineFront {
    /// Where a turn's stream goes.
    surface: Surface,
    /// How a settled turn's own line is rendered, or `None` for a surface that
    /// words the class itself: `saya ask` states its own exit-code wording
    /// (`commands/query.rs`), so a turn that said its line here would say it
    /// twice.
    wording: Option<RenderFormat>,
    /// Whether this surface may read an answer from stdin at all: the plain
    /// loop's live-terminal fact. A piped run denies every question unasked.
    can_prompt: bool,
    /// The tally every turn's sink records its tool events into, read by the
    /// scripted run's one terminal result.
    tally: TallyHandle,
    /// The run's one stop path: the sample a Ctrl-C settles, and the counters
    /// every settled turn's metrics section prints.
    signals: Signals,
    /// The turn now running, read for the session as it is rendered: its usage
    /// and the words it says.
    stream: Arc<TurnStream>,
    shared: Mutex<Shared>,
}

impl LineFront {
    pub(crate) fn new(format: RenderFormat, can_prompt: bool, tally: TallyHandle) -> Self {
        Self {
            surface: Surface::Terminal(format),
            wording: Some(format),
            can_prompt,
            tally,
            signals: Signals::new(None),
            stream: Arc::new(TurnStream::default()),
            shared: Mutex::new(Shared {
                turn: CancellationToken::new(),
                ended: None,
            }),
        }
    }

    /// The one-shot `ask` surface's front: the run's stream over the answer sink
    /// the surface brought, and none of a settled turn's own wording — the ask
    /// states the class and says the line for it, once, from the exit-code
    /// table.
    pub(crate) fn over(
        sink: Arc<dyn AgentEventSink>,
        can_prompt: bool,
        tally: TallyHandle,
    ) -> Self {
        Self {
            surface: Surface::Given(sink),
            wording: None,
            ..Self::new(RenderFormat::Text, can_prompt, tally)
        }
    }

    /// The same front recording the session's answering scope: its counters are
    /// the `--verbose` section a settled turn prints, and its accepted stop is
    /// what the settlement sample is measured from.
    pub(crate) fn with_scope(mut self, scope: QuestionScope) -> Self {
        self.signals = Signals::new(Some(scope));
        self
    }

    /// The turn now running: a pending question ends when its turn does.
    pub(crate) fn set_turn(&self, turn: CancellationToken) {
        self.lock().turn = turn;
    }

    /// The operator's stop: this turn's own token and the accepted stop
    /// instant — never the run's, so the next line is a turn of the same run.
    pub(crate) fn stop_turn(&self) {
        let turn = self.lock().turn.clone();
        turn.cancel();
        self.signals.request_stop();
    }

    /// How the last settled turn ended, taken once — the loop above reads it
    /// to word its exit status and to settle the session record.
    pub(crate) fn take_outcome(&self) -> Option<ChatOutcome> {
        self.lock().ended.take()
    }

    /// What the last settled turn said and spent, taken once beside its
    /// outcome: the session folds it into its own accumulator.
    pub(crate) fn take_record(&self) -> TurnRecord {
        self.stream.take()
    }

    /// The tool tally every turn's sink recorded into: the scripted run's one
    /// terminal result reports it.
    pub(crate) fn tally(&self) -> crate::interactive::turn_summary::ToolTally {
        self.tally.lock().expect("tool tally").clone()
    }

    /// Says one settled turn in today's wording.
    fn say(&self, outcome: &ChatOutcome) {
        // The settle records the one cancellation sample a Ctrl-C was waiting
        // for, then prints the section — the order the old turn body used.
        self.signals.settle();
        let Some(format) = self.wording else {
            return;
        };
        if let Some(event) = wording(outcome) {
            let rendered = render_event(&event, format);
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
        let terminal: Arc<dyn AgentEventSink> = match &self.surface {
            Surface::Terminal(format) => {
                Arc::new(TerminalSink::new(*format).with_tally(Arc::clone(&self.tally)))
            }
            Surface::Given(sink) => Arc::clone(sink),
        };
        // The same stream, read for the session before it is rendered: the
        // turn's usage and the words it says.
        Arc::new(StreamSink::new(terminal, Arc::clone(&self.stream)))
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
                let format = self.wording.unwrap_or(RenderFormat::Text);
                let rendered = render_event(&TerminalEvent::Error { message }, format);
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
