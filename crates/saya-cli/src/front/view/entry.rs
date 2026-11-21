//! The transcript's entries: the presentation-free data one turn's event
//! stream folds into — a turn reads as the operator's message, one activity
//! block with a line per tool step, the answer and the result preview, and the
//! other blocks of `02` §3.4-§3.9 in [`super::entry_blocks`].
//! Nothing here names a glyph, a colour or a widget.

use serde_json::Value;

use super::entry_blocks::{Clarification, Command, Edit, Error, Helper, Stop};
use super::entry_result::ResultPreview;

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

    /// The turn at `index`, to be changed. This is the one write that reaches
    /// back into an older turn, which is what the block cursor's fold needs: a
    /// group in *any* turn opens, not only the newest (`02` §6).
    pub(crate) fn turn_mut(&mut self, index: usize) -> Option<&mut TurnView> {
        self.turns.get_mut(index)
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

    /// The block at `index`, to be changed.
    pub(crate) fn entry_mut(&mut self, index: usize) -> Option<&mut Entry> {
        self.entries.get_mut(index)
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
    /// An edit of one file, and the diff of it (`02` §3.4).
    Edit(Edit),
    /// A command the turn ran, and how it ended (`02` §3.5).
    Command(Command),
    /// A helper's work, as one block (`02` §3.6).
    Helper(Helper),
    /// What ended the turn in failure (`02` §3.7).
    Error(Error),
    /// What stopped the turn, or the operator's cancel (`02` §3.8).
    Stop(Stop),
    /// The model's clarifying question (`02` §3.9).
    Question(Clarification),
}

impl Entry {
    /// The block's place in the turn: message, the work the turn did, answer,
    /// preview, then what the session says or what ended the turn.
    const fn rank(&self) -> u8 {
        match self {
            Self::UserMessage(_) => 0,
            Self::Activity(_) | Self::Edit(_) | Self::Command(_) | Self::Helper(_) => 1,
            Self::Answer(_) => 2,
            Self::ResultPreview(_) => 3,
            Self::Notice(_) | Self::Error(_) | Self::Stop(_) | Self::Question(_) => 4,
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

/// Something the session itself has to say, in its own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    pub(crate) message: String,
}
