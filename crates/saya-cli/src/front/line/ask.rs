//! The line front's one question: today's terminal approval prompt.
//!
//! The operator states a call as its own facts — the command and where it runs,
//! the query and its connection — sanitised and bounded before it reaches this
//! far. This puts them on stderr with the same answers line every prompt of the
//! crate states, reads one typed answer, and maps it with the same
//! [`terminal_choice`] the per-call decider uses: one vocabulary, one wording,
//! so a script that answers `s` means the same thing on both surfaces.
//!
//! A surface with nobody to ask — a pipe, a front with no terminal — denies
//! without waiting, which is what the terminal prompt itself does when stdin is
//! not a terminal.

use saya_agent::CancellationToken;

use crate::agent::tools::consent::Answer;
use crate::chat::Question;
use crate::grant_token::session_answers_line;
use crate::prompt_approval::terminal_choice;

/// Puts one call to the person at the terminal and returns their answer.
pub(super) async fn put(can_prompt: bool, turn: &CancellationToken, question: &Question) -> Answer {
    if !can_prompt {
        return Answer::Deny;
    }
    eprint!(
        "{}\n{} ",
        question.facts,
        session_answers_line(question.grant.as_deref())
    );
    let _ = std::io::Write::flush(&mut std::io::stderr());
    let Ok(Some(answer)) = crate::terminal_input::approval_line(turn).await else {
        return Answer::Deny;
    };
    // The turn may have stopped while the answer was being typed: the call it
    // was about must not run on it.
    if turn.is_cancelled() {
        return Answer::Deny;
    }
    match terminal_choice(&answer, question.grant.as_deref()) {
        saya_agent::ApprovalChoice::AllowOnce => Answer::Once,
        saya_agent::ApprovalChoice::AllowSession { .. } => Answer::Session,
        saya_agent::ApprovalChoice::Deny => Answer::Deny,
    }
}
