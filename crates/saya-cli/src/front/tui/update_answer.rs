//! The card's answers, carried out: the call an operator consented to, and the
//! press that named it.
//!
//! Kept apart from [`super::update`] so the surface's one decision function
//! stays a list of what a message means rather than what an answer *is*: an
//! answer the card offers is read here, from the same table the panel paints
//! ([`super::cards_keys`]), never spelled a second time.

use super::cards_keys;
use super::model::Model;
use super::msg::Cmd;
use crate::agent::tools::consent::Answer;

/// One answer to the open card: the call is answered, and the turn that asked
/// keeps going. An approval is the operator's consent, so the answer is
/// carried out exactly as it was given — once, for the session, or never.
pub(super) fn answer_card(model: &mut Model, given: Answer, cmds: &mut Vec<Cmd>) {
    if let Some(card) = model.take_card() {
        cmds.push(Cmd::Answer(card.id, given));
        // One card at a time: the job that owed this call no longer waits, and
        // the ask behind it takes the slot it left (`02` §5, T9/A5) — and rings
        // the terminal, exactly as it did when it arrived.
        if model.answered(card.id) {
            cmds.push(Cmd::Bell);
        }
    }
}

/// The answer the card offers at `index`, as the keys that answer it would give
/// it: `y` once, `s` the session grant it named, `n` and Esc the refusal.
pub(super) fn answer_at(model: &Model, index: usize) -> Option<Answer> {
    let question = model.card()?;
    let answers = cards_keys::answers(question);
    let choice = answers.get(index)?;
    cards_keys::answer_of(choice.key)
}
