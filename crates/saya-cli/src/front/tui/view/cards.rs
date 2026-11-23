//! The panel docked above the composer: the call the operator has to answer,
//! as the rows a pane paints.
//!
//! Strings only — no widget, no colour, no clock — so a test asserts the words
//! without a terminal (the same rule the transcript's `steps.rs` follows).
//!
//! One thing this module must never do is reword the call: the block is the
//! question's own bytes, line for line, because the operator is answering for
//! the bytes they read and nothing else. What the module decides is how loudly
//! that call is put — a title that names what is being allowed, a place line,
//! and the answers one per line — never what is asked or what an answer grants.

use super::cards_body::{self, Parts};
use super::cards_choices;
use super::cards_family;
use crate::chat::Question;

use crate::interactive::tui::wrap::cell_width;
pub(crate) use cards_choices::choices as choices_of;
pub(crate) use cards_family::family as family_of;

/// The most body rows a docked card shows at the design's 80x24: the panel's
/// chrome — title, place, answers and borders — takes the rows above and below,
/// and the call itself gets six.
pub(crate) const BODY_ROWS: usize = 6;

/// The rows a card's chrome needs on a screen, so the body gets what is left.
const CHROME_ROWS: usize = 18;

/// What the panel's last row says the keys do, while the panel answers.
pub(crate) const HINT: &str = "↑↓ choose · Enter answer · v full details";

/// What the panel's last row says while the composer holds the keyboard: the
/// one key that gives it to the panel, and the one that opens the details.
pub(crate) const HINT_UNFOCUSED: &str = "Tab to answer · v full details";

/// Which of the three cards this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A call with an effect, put to the operator.
    Approval,
    /// The plan gate's ask: the plan is the block, and the answers open or
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

/// One answer the panel offers: its key, the words beside it, and the shorter
/// words the composer's hint row gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Choice {
    /// The one press that answers this, as the panel writes it (`y`, `Esc`).
    pub(crate) key: &'static str,
    /// What the press does, and — for a session grant — what it covers.
    pub(crate) label: String,
    /// The same answer in the few words a one-row hint has room for.
    pub(crate) short: &'static str,
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
    pub(super) target: Vec<String>,
    /// The change the call makes to a file, as the bounded diff rows the panel
    /// paints under the path — built from the call's own bytes before it runs.
    /// Empty for a call that changes no file text.
    pub(super) change: Vec<String>,
    /// The call's exact machine bytes — a job's proposal line — which belong
    /// behind `v`, not in the body an operator reads at a glance.
    pub(super) details: Vec<String>,
    place: Option<String>,
    choices: Vec<Choice>,
    /// The body rows this card may show before `v` opens the rest.
    pub(super) cap: usize,
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

    /// The title: what is being allowed, and which of how many this is.
    pub(crate) fn title(&self) -> &str {
        &self.title
    }

    /// The call's own bytes, verbatim.
    pub(crate) fn target(&self) -> &[String] {
        &self.target
    }

    /// The change the call makes to a file, one row per line, bounded: the
    /// `@@` hunk headers, the rows it takes away and the rows it adds — or a
    /// single size row for a change too large or not text.
    pub(crate) fn change(&self) -> &[String] {
        &self.change
    }

    /// The exact bytes the card holds behind `v`: the proposal a job's consent
    /// covers, never a line of the body it reads first.
    pub(crate) fn details(&self) -> &[String] {
        &self.details
    }

    /// Where the call runs, when the panel can say.
    pub(crate) fn place(&self) -> Option<&str> {
        self.place.as_deref()
    }

    /// The answers the card offers, in the order it offers them.
    pub(crate) fn choices(&self) -> &[Choice] {
        &self.choices
    }

    /// What the panel's last row says the keys do, read by the seat that has
    /// the keyboard.
    pub(crate) const fn hint(&self, focused: bool) -> &'static str {
        if focused { HINT } else { HINT_UNFOCUSED }
    }

    /// The answer field's placeholder, on a card whose answer is typed.
    pub(crate) fn field(&self) -> Option<&str> {
        if self.kind == Kind::Question {
            Some("Type a number or your own answer")
        } else {
            None
        }
    }
}

/// The approval card for `question`: its family's title, the call's own bytes,
/// where it runs, and the answers it offers.
pub(crate) fn approval(question: &Question, folder: Option<&str>, screen_rows: u16) -> Card {
    approval_waiting(question, folder, screen_rows, 1)
}

/// The approval card of a call that is one of `waiting`, oldest on the card.
pub(crate) fn approval_waiting(
    question: &Question,
    folder: Option<&str>,
    screen_rows: u16,
    waiting: usize,
) -> Card {
    let family = cards_family::family(question);
    let Parts {
        target,
        details,
        in_facts,
    } = cards_body::parts(&question.facts, family);
    Card {
        kind: kind_of(family),
        family,
        title: cards_family::title(family, waiting.max(1)),
        place: cards_family::place(family, in_facts, folder),
        target,
        change: question.change.clone(),
        details,
        choices: cards_choices::choices(question, family),
        cap: body_cap(screen_rows),
    }
}

/// The clarification card for `text`: the model's question as the block and the
/// field the answer is typed into — the answer is the next turn, so no key of
/// the card's own grants anything.
pub(crate) fn clarification(text: &str, screen_rows: u16) -> Card {
    Card {
        kind: Kind::Question,
        family: Family::Other,
        title: String::from("Question"),
        target: text.split('\n').map(str::to_owned).collect(),
        change: Vec::new(),
        details: Vec::new(),
        place: None,
        choices: Vec::new(),
        cap: body_cap(screen_rows),
    }
}

/// The body rows a card of `screen_rows` total rows may spend: what the panel's
/// chrome leaves, and never more than [`BODY_ROWS`]. At 80x24 that is exactly
/// the design's six.
fn body_cap(screen_rows: u16) -> usize {
    (screen_rows as usize)
        .saturating_sub(CHROME_ROWS)
        .clamp(1, BODY_ROWS)
}

/// Which card a family is: the plan gate's ask has answers of its own.
fn kind_of(family: Family) -> Kind {
    if family == Family::Plan {
        Kind::Plan
    } else {
        Kind::Approval
    }
}
