//! The answers a card offers, in the order it offers them.
//!
//! The keys and what they mean are today's (`interactive/tui/keys/approvals`);
//! what changes here is how they are written: one answer per line, each with
//! the press that gives it, so the panel reads as a list rather than a row. A
//! session grant appears only where the call named a token — the token is the
//! offer, and the panel says what it covers.

use super::cards::{Choice, Family};
use crate::chat::Question;

/// The plan gate's yes: it opens the write surface, and is not a call's grant.
const APPROVE_PLAN: &str = "Approve plan";
/// The plan gate's no: the plan keeps being worked on.
const KEEP_PLANNING: &str = "Keep planning";
/// The one-call yes.
const ALLOW_ONCE: &str = "Allow once";
/// The refusal, spelled for the answer that grants nothing.
const DENY: &str = "Deny";
/// How a session answer states what it covers.
const COVERS: &str = "Allow for this session";

/// The answers `question` offers, given the family it belongs to.
pub(crate) fn choices(question: &Question, family: Family) -> Vec<Choice> {
    if family == Family::Plan {
        return vec![
            choice("y", APPROVE_PLAN),
            choice("n", KEEP_PLANNING),
            // Esc is the refusal of every card, and a plan's refusal is the
            // decision to keep planning: no answer of this card's is Deny.
            choice("Esc", KEEP_PLANNING),
        ];
    }
    let mut list = vec![choice("y", ALLOW_ONCE)];
    if let Some(token) = question.grant.as_deref() {
        list.push(choice("s", &format!("{COVERS} · {token}")));
    }
    list.push(choice("n", DENY));
    list.push(choice("Esc", DENY));
    list
}

/// One answer: its key and its words.
fn choice(key: &'static str, label: &str) -> Choice {
    Choice {
        key,
        label: label.to_owned(),
    }
}
