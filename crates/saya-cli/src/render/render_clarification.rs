//! Text shaping for [`AgentEvent::ClarificationNeeded`] (B3c).
//!
//! The wording here is shared by the headless path ([`super::TerminalEvent`]
//! → [`super::render_event`]) and the TUI path ([`crate::interactive::tui`]
//! `apply_event`, which pushes it as the transcript's Question block) so both
//! surfaces say the same thing in the same words. The block is deliberately
//! distinct from an answer: the model asked instead of assuming, and the
//! user's next message answers it — nothing here is a result.

/// Shapes the question block for one clarification: the head line names the
/// pause and carries the question; the options, when present, are numbered —
/// so the user can answer by number or in their own words. Never empty: an
/// event with a question always renders something.
pub(crate) fn clarification_text(question: &str, options: &[String]) -> String {
    let mut out = format!("⏸ question · {question}\n");
    for (index, option) in options.iter().enumerate() {
        out.push_str(&format!("  {}. {option}\n", index + 1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The block shape: the head carries the question, the options are
    /// numbered from 1.
    #[test]
    fn the_block_carries_the_question_and_numbers_the_options() {
        let text = clarification_text(
            "Which metric should \"active users\" use?",
            &[
                "sessions in the last 30 days".to_string(),
                "purchases in the last 90 days".to_string(),
            ],
        );
        assert_eq!(
            text,
            "⏸ question · Which metric should \"active users\" use?\n  \
             1. sessions in the last 30 days\n  2. purchases in the last 90 days\n"
        );
    }

    /// No options: the question alone, no numbering, no trailing blank lines.
    #[test]
    fn without_options_the_question_stands_alone() {
        assert_eq!(
            clarification_text("Which table holds active users?", &[]),
            "⏸ question · Which table holds active users?\n"
        );
    }
}
