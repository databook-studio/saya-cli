//! The card docked above the composer: the call the operator has to answer,
//! as the rows a pane paints.
//!
//! Strings only — no widget, no colour, no clock — so a test asserts the words
//! without a terminal (the same rule the transcript's `steps.rs` follows).
//!
//! One thing this module must never do is reword the call: the body is the
//! question's own facts, line for line, because the operator is answering for
//! the bytes they read and nothing else. The card decides how much of that
//! body is on screen — six rows at the design's 80x24, `v` opens the rest —
//! and never what it says.
//!
//! Three cards come from two shapes. An approval names its caller and carries
//! the call's facts; the plan gate's ask is the same card, with the plan as its
//! body and the two answers that decision has. A clarification is the one card
//! whose answer is typed, so it offers a field instead of a grant.

use saya_agent::PermissionMode;

use crate::agent::tools::coding::plan_gate;
use crate::agent::tools::consent::Caller;
use crate::chat::Question;
use crate::interactive::session::permissions::mode::name;
use crate::interactive::tui::wrap::cell_width;

/// The most body rows a docked card shows at the design's 80x24: the card's
/// chrome — title, why it is asked, keys and borders — takes the rows above
/// and below, and the facts get six (`02` §5).
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

/// One docked card, as the rows a pane paints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Card {
    kind: Kind,
    title: String,
    /// The body, verbatim and complete — the clip is a paint decision
    /// ([`Card::shown`]), never a rewrite.
    body: Vec<String>,
    because: Option<String>,
    /// The answer field of a clarification card, as its placeholder.
    field: Option<String>,
    keys: String,
    /// The keys row's shorter spellings, for a terminal narrower than the full
    /// row: the compact form drops the answer verbs, the terse one the grant's
    /// token. A key never leaves the row (`02` §5).
    compact: String,
    terse: String,
    /// The body rows this card may show before `v` opens the rest.
    cap: usize,
}

impl Card {
    /// Which card this is.
    pub(crate) const fn kind(&self) -> Kind {
        self.kind
    }

    /// The title: the card's own word and the caller's line.
    pub(crate) fn title(&self) -> &str {
        &self.title
    }

    /// The whole body, verbatim.
    pub(crate) fn body(&self) -> &[String] {
        &self.body
    }

    /// Why the operator is being asked, when the card can say.
    pub(crate) fn because(&self) -> Option<&str> {
        self.because.as_deref()
    }

    /// The answer field's placeholder, on a card whose answer is typed.
    pub(crate) fn field(&self) -> Option<&str> {
        self.field.as_deref()
    }

    /// The keys row.
    pub(crate) fn keys(&self) -> &str {
        &self.keys
    }

    /// The keys row fitted to `width` cells: the full words when they fit, then
    /// the compact spelling, then the one without the grant's token. The row
    /// shortens in words; no key is ever cut off it.
    pub(crate) fn keys_for(&self, width: usize) -> &str {
        if cell_width(&self.keys) <= width {
            &self.keys
        } else if cell_width(&self.compact) <= width {
            &self.compact
        } else {
            &self.terse
        }
    }

    /// The body rows this card shows: all of them once the operator asked for
    /// the whole body (`v`), and never more than the cap before that. The rows
    /// are a prefix of [`Card::body`], so what is shown is what was said.
    pub(crate) fn shown(&self, full: bool) -> &[String] {
        if full {
            return &self.body;
        }
        let shown = self.cap.min(self.body.len());
        &self.body[..shown]
    }

    /// How many body rows the cap is withholding, and so whether `v` has
    /// anything to open.
    pub(crate) fn withheld(&self) -> usize {
        self.body.len().saturating_sub(self.cap)
    }
}

/// The approval card for `question`: its caller's line as the title, the facts
/// verbatim as the body, why it is asked, and the keys that answer it. The plan
/// gate's ask is the same card with the plan as its body and the two answers
/// that decision has.
pub(crate) fn approval(question: &Question, mode: PermissionMode, screen_rows: u16) -> Card {
    let kind = kind_of(question);
    let body = rows(&question.facts);
    let cap = body_cap(screen_rows);
    let withheld = body.len() > cap;
    let (title, because, keys, compact, terse) = match kind {
        Kind::Plan => {
            let keys = plan_keys(withheld);
            let compact = plan_keys_compact(withheld);
            (
                format!("Approve plan? · {}", question.caller),
                None,
                keys.clone(),
                compact.clone(),
                compact,
            )
        }
        Kind::Approval => (
            format!("Approve? · {}", question.caller),
            Some(because(mode, question)),
            approval_keys(question.grant.as_deref(), withheld),
            approval_compact(question.grant.as_deref(), withheld),
            approval_terse(question.grant.as_deref(), withheld),
        ),
        // A clarification is never built from a `Question`: its text is the
        // model's own words, not a tool call's facts.
        Kind::Question => (
            String::from("Question"),
            None,
            String::new(),
            String::new(),
            String::new(),
        ),
    };
    Card {
        kind,
        title,
        body,
        because,
        field: None,
        keys,
        compact,
        terse,
        cap,
    }
}

/// The clarification card for `text`: the model's question as the body and the
/// field the answer is typed into — the answer is the next turn, so no key of
/// the card's own grants anything.
pub(crate) fn clarification(text: &str, screen_rows: u16) -> Card {
    Card {
        kind: Kind::Question,
        title: String::from("Question"),
        body: rows(text),
        because: None,
        field: Some(String::from("Type a number or your own answer")),
        keys: String::new(),
        compact: String::new(),
        terse: String::new(),
        cap: body_cap(screen_rows),
    }
}

/// The body rows a card of `screen_rows` total rows may spend: what the card's
/// chrome leaves, and never more than [`BODY_ROWS`]. At 80x24 that is exactly
/// the design's six.
fn body_cap(screen_rows: u16) -> usize {
    (screen_rows as usize)
        .saturating_sub(CHROME_ROWS)
        .clamp(1, BODY_ROWS)
}

/// Which card an approval-shaped question is: the plan gate's ask is the one
/// ask in this crate whose caller line is not a call's, so it is the one the
/// card can tell from an approval without the chat contract carrying a kind.
fn kind_of(question: &Question) -> Kind {
    if question.caller.contains(&plan_line()) {
        Kind::Plan
    } else {
        Kind::Approval
    }
}

/// The "asked because:" line: the mode in force and the grant this call has
/// none of, in the words the approval frontends already use for both.
fn because(mode: PermissionMode, question: &Question) -> String {
    match question.grant.as_deref() {
        Some(token) => format!("asked because: mode {} · no grant for {token}", name(mode)),
        None => format!(
            "asked because: mode {} · no session grant for this tool",
            name(mode)
        ),
    }
}

/// The approval's keys row: today's answers (`y`/`a` once, `s` the session
/// token, `n`/`d`/Esc deny) and `v` for the body the cap withheld. A call with
/// no token offers no grant, exactly as the terminal prompt does not.
fn approval_keys(grant: Option<&str>, withheld: bool) -> String {
    let mut row = String::from("y allow once");
    if let Some(token) = grant {
        row.push_str(&format!("   s allow {token} this session"));
    }
    row.push_str("   n deny");
    row.push_str(view_hint(withheld));
    row
}

/// The plan's keys row: the two answers opening or keeping closed the write
/// surface, and `v` for a plan longer than the card.
fn plan_keys(withheld: bool) -> String {
    format!(
        "y approve plan · n keep planning{}",
        if withheld { "   v view all" } else { "" }
    )
}

/// The plan's keys row at a narrow width: the same words, in one spelling, so
/// the compact form is the full one.
fn plan_keys_compact(withheld: bool) -> String {
    format!(
        "y approve plan   n keep planning{}",
        if withheld { "   v view all" } else { "" }
    )
}

/// The approval's keys row at a narrow width: the answers' own verbs go, the
/// keys and the grant stay.
fn approval_compact(grant: Option<&str>, withheld: bool) -> String {
    let mut row = String::from("y allow once");
    if let Some(token) = grant {
        row.push_str(&format!("   s {token} session"));
    }
    row.push_str("   n deny");
    row.push_str(view_hint(withheld));
    row
}

/// The approval's keys row at the narrowest width: the grant's token goes too,
/// because a key that answers is worth more than the word it records.
fn approval_terse(grant: Option<&str>, withheld: bool) -> String {
    let token = grant.map(|_| "   s session").unwrap_or_default();
    format!("y allow once{token}   n deny{}", view_hint(withheld))
}

/// The `v` hint. `v` always works — it opens the whole body — and says so
/// with `all` exactly when the cap is hiding rows.
fn view_hint(withheld: bool) -> &'static str {
    if withheld {
        "   v view all"
    } else {
        "   v view"
    }
}

/// `text` as body rows, verbatim: one row per line, empty lines kept, so the
/// rows a card holds join back into the bytes the operator is answering for.
fn rows(text: &str) -> Vec<String> {
    text.split('\n').map(str::to_owned).collect()
}

/// The plan gate's line, as the card reads it: the one ask in this crate whose
/// caller line is not a call's, so the card can tell a plan from an approval
/// without the chat contract carrying a kind.
fn plan_line() -> String {
    plan_gate::line(&Caller::Session)
}
