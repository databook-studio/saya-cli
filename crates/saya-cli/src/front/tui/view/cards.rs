//! The card docked above the composer: the call the operator has to answer,
//! as the rows a pane paints.
//!
//! Strings only — no widget, no colour, no clock — so a test asserts the words
//! without a terminal (the same rule the transcript's `steps.rs` follows).
//!
//! One thing this module must never do is reword the call: the body is the
//! question's own facts, line for line, because the operator is answering for
//! the bytes they read and nothing else.

use crate::agent::tools::coding::plan_gate;
use crate::agent::tools::consent::Caller;
use crate::chat::Question;
use crate::interactive::tui::wrap::cell_width;

/// The most body rows a docked card shows at the design's 80x24.
pub(crate) const BODY_ROWS: usize = 6;

/// The rows a card's chrome needs on a screen, so the body gets what is left.
const CHROME_ROWS: usize = 18;

/// Which of the three cards this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A call with an effect, put to the operator.
    Approval,
    /// The plan gate's ask: the plan is the body, and the answers open or
    /// keep closed the write surface.
    Plan,
    /// A clarification: the model's own question, answered by typing.
    Question,
}

/// The family a call belongs to, as the panel's title names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    /// A host command.
    Command,
    /// A change to a file in the workspace.
    FileChange,
    /// A read-only query on a connection.
    Query,
    /// A request that leaves this machine.
    Web,
    /// The plan gate's ask.
    Plan,
    /// A coding job's start consent.
    Scope,
    /// A call the panel cannot place.
    Other,
}

/// One answer the panel offers: its key, and the words beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Choice {
    /// The one press that answers this, as the panel writes it (`y`, `Esc`).
    pub(crate) key: &'static str,
    /// What the press does, and — for a session grant — what it covers.
    pub(crate) label: String,
}

/// The cells `text` occupies on a terminal, as the panes measure.
pub(crate) fn cells(text: &str) -> usize {
    cell_width(text)
}

/// One docked card, as the rows a pane paints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Card {
    kind: Kind,
    family: Family,
    title: String,
    /// The call's own bytes, verbatim and complete.
    body: Vec<String>,
    place: Option<String>,
    choices: Vec<Choice>,
    /// The body rows this card may show before `v` opens the rest.
    cap: usize,
}

impl Card {
    /// Which card this is.
    pub(crate) const fn kind(&self) -> Kind {
        self.kind
    }

    /// Which family of call this is.
    pub(crate) const fn family(&self) -> Family {
        self.family
    }

    /// The title.
    pub(crate) fn title(&self) -> &str {
        &self.title
    }

    /// The call's own bytes, verbatim.
    pub(crate) fn target(&self) -> &[String] {
        &self.body
    }

    /// Where the call runs, when the card can say.
    pub(crate) fn place(&self) -> Option<&str> {
        self.place.as_deref()
    }

    /// The answers the card offers, in the order it offers them.
    pub(crate) fn choices(&self) -> &[Choice] {
        &self.choices
    }

    /// The rows the body shows: `full` shows all of it.
    pub(crate) fn shown_rows(&self, full: bool, _room: usize) -> Vec<String> {
        if full {
            return self.body.clone();
        }
        self.body[..self.cap.min(self.body.len())].to_vec()
    }

    /// How many body rows the cap withholds.
    pub(crate) fn withheld(&self, _room: usize) -> usize {
        self.body.len().saturating_sub(self.cap)
    }

    /// The answer field of a clarification card, as its placeholder.
    pub(crate) fn field(&self) -> Option<&str> {
        if self.kind == Kind::Question {
            Some("Type a number or your own answer")
        } else {
            None
        }
    }
}

/// The approval card for `question`.
pub(crate) fn approval(question: &Question, _folder: Option<&str>, screen_rows: u16) -> Card {
    let kind = kind_of(question);
    let title = match kind {
        Kind::Plan => format!("Approve plan? · {}", question.caller),
        Kind::Approval => format!("Approve? · {}", question.caller),
        Kind::Question => String::from("Question"),
    };
    Card {
        kind,
        family: Family::Other,
        title,
        body: rows(&question.facts),
        place: None,
        choices: Vec::new(),
        cap: body_cap(screen_rows),
    }
}

/// The approval card of a call that is one of `waiting` in the queue.
pub(crate) fn approval_waiting(
    question: &Question,
    folder: Option<&str>,
    screen_rows: u16,
    _waiting: usize,
) -> Card {
    approval(question, folder, screen_rows)
}

/// The clarification card for `text`.
pub(crate) fn clarification(text: &str, screen_rows: u16) -> Card {
    Card {
        kind: Kind::Question,
        family: Family::Other,
        title: String::from("Question"),
        body: rows(text),
        place: None,
        choices: Vec::new(),
        cap: body_cap(screen_rows),
    }
}

/// The body rows a card of `screen_rows` total rows may spend.
fn body_cap(screen_rows: u16) -> usize {
    (screen_rows as usize)
        .saturating_sub(CHROME_ROWS)
        .clamp(1, BODY_ROWS)
}

/// Which card an approval-shaped question is.
fn kind_of(question: &Question) -> Kind {
    if question.caller.contains(&plan_line()) {
        Kind::Plan
    } else {
        Kind::Approval
    }
}

/// `text` as body rows, verbatim: one row per line, empty lines kept.
fn rows(text: &str) -> Vec<String> {
    text.split('\n').map(str::to_owned).collect()
}

/// The plan gate's line, as the card reads it.
fn plan_line() -> String {
    plan_gate::line(&Caller::Session)
}
