//! The transcript's entries: the presentation-free data one turn's event
//! stream folds into — a turn reads as the operator's message, one activity
//! block with a line per tool step, the answer and the result preview.
//! Nothing here names a glyph, a colour or a widget.

use saya_types::QueryResult;
use serde_json::Value;

use crate::agent::tools::{CaptureEvent, CaptureRefusalReason};

/// The conversation as its front holds it: one view per turn, oldest first.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct TranscriptView {
    turns: Vec<TurnView>,
}

impl TranscriptView {
    /// An empty conversation.
    pub(crate) const fn new() -> Self {
        Self { turns: Vec::new() }
    }

    /// The turns, oldest first.
    pub(crate) fn turns(&self) -> &[TurnView] {
        &self.turns
    }

    /// Opens a turn with the operator's own words — the surface's, never the
    /// event stream's.
    pub(crate) fn begin_turn(&mut self, message: impl Into<String>) {
        self.push_turn();
        self.current().push(Entry::UserMessage(UserMessage {
            text: message.into(),
        }));
    }

    /// The turn an event folds into: the last one, or a fresh one when none is
    /// open yet — a notice can arrive before the first message.
    pub(crate) fn current(&mut self) -> &mut TurnView {
        if self.turns.is_empty() {
            self.push_turn();
        }
        let last = self.turns.len() - 1;
        &mut self.turns[last]
    }

    /// Appends one empty turn.
    fn push_turn(&mut self) {
        self.turns.push(TurnView::default());
    }
}

/// One turn of the conversation.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct TurnView {
    entries: Vec<Entry>,
}

impl TurnView {
    /// The turn's blocks, in reading order.
    pub(crate) fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Files one block at its place in the turn, so a block's arrival on the
    /// stream never decides the order the turn reads in.
    pub(crate) fn push(&mut self, entry: Entry) {
        let place = self
            .entries
            .iter()
            .position(|held| held.rank() > entry.rank())
            .unwrap_or(self.entries.len());
        self.entries.insert(place, entry);
    }

    /// The turn's activity block, once one of its tool calls opened it.
    pub(crate) fn activity_mut(&mut self) -> Option<&mut Activity> {
        self.entries.iter_mut().find_map(Entry::activity_mut)
    }

    /// The turn's answer, once the model has streamed text.
    pub(crate) fn answer_mut(&mut self) -> Option<&mut Answer> {
        self.entries.iter_mut().find_map(Entry::answer_mut)
    }
}

/// One block of a turn.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Entry {
    /// The operator's own words, opening the turn.
    UserMessage(UserMessage),
    /// The turn's tool steps as one block, folded once a successful turn ends.
    Activity(Activity),
    /// The model's answer, as its text streams in.
    Answer(Answer),
    /// The result the answer rests on.
    ResultPreview(ResultPreview),
    /// Something the session itself has to say.
    Notice(Notice),
}

impl Entry {
    /// The block's place in the turn: message, activity, answer, preview, then
    /// whatever the session says after it.
    const fn rank(&self) -> u8 {
        match self {
            Self::UserMessage(_) => 0,
            Self::Activity(_) => 1,
            Self::Answer(_) => 2,
            Self::ResultPreview(_) => 3,
            Self::Notice(_) => 4,
        }
    }

    /// This entry read as an activity block.
    pub(crate) fn activity(&self) -> Option<&Activity> {
        match self {
            Self::Activity(activity) => Some(activity),
            _ => None,
        }
    }

    fn activity_mut(&mut self) -> Option<&mut Activity> {
        match self {
            Self::Activity(activity) => Some(activity),
            _ => None,
        }
    }

    fn answer_mut(&mut self) -> Option<&mut Answer> {
        match self {
            Self::Answer(answer) => Some(answer),
            _ => None,
        }
    }
}

/// The operator's own words, opening a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UserMessage {
    pub(crate) text: String,
}

/// The turn's single activity block: one line per tool step, in call order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Activity {
    pub(crate) steps: Vec<Step>,
    /// Whether the block paints as one summary line.
    pub(crate) folded: bool,
    /// Whether a step failed. A failed block keeps every line open: what went
    /// wrong is the part the operator has to read.
    pub(crate) failed: bool,
}

impl Activity {
    /// A block as the turn's first tool call opens it: no steps, live, sound.
    pub(crate) const fn live() -> Self {
        Self {
            steps: Vec::new(),
            folded: false,
            failed: false,
        }
    }
}

/// One tool call of the activity block.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Step {
    /// The tool's name, as the model called it.
    pub(crate) tool: String,
    /// The call's raw payload, so a step can name what it ran.
    pub(crate) arguments: Value,
    pub(crate) state: StepState,
}

/// How one step stands.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StepState {
    /// Called, with no outcome yet.
    Running,
    Done {
        summary: String,
    },
    Failed {
        summary: String,
    },
    Denied {
        reason: String,
    },
}

impl StepState {
    /// Whether the step is still awaiting its outcome.
    pub(crate) const fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }
}

/// The model's answer, as its text streams in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Answer {
    pub(crate) text: String,
}

/// The result one successful query handed the front: the rows the model saw,
/// or the refusal that kept them from it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResultPreview {
    /// The connection that actually ran the query.
    pub(crate) connection: String,
    pub(crate) sql: String,
    pub(crate) result: QueryResult,
    /// Why the capture was refused: a preview with no rows says so, and never
    /// pretends the result was empty.
    pub(crate) refusal: Option<CaptureRefusalReason>,
}

impl ResultPreview {
    /// The preview of one capture event, held or refused.
    pub(crate) fn from_capture(event: &CaptureEvent) -> Self {
        match event {
            CaptureEvent::Captured(capture) => Self {
                connection: capture.connection.clone(),
                sql: capture.sql.clone(),
                result: capture.result.clone(),
                refusal: None,
            },
            CaptureEvent::Refused {
                sql,
                connection,
                reason,
            } => Self {
                connection: connection.clone(),
                sql: sql.clone(),
                result: QueryResult::empty(sql),
                refusal: Some(*reason),
            },
        }
    }
}

/// Something the session itself has to say, in its own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    pub(crate) message: String,
}
