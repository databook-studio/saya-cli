//! The inspector line (`02` §6, §3.2): the provenance of the entry the block
//! cursor holds, one dim row under it, shown while `i` is on.
//!
//! `02` §3.2 draws it as `demo · exec xmwozaj7b-4 · model saw first 50 rows`
//! and §6 names the facts it carries: the execution id, the row cap, the
//! connection and the timing. The surface can prove some of those today —
//! which connection ran a result and how many rows it held, how a command
//! ended and how long it took — because the view model T1 folds holds no
//! execution evidence yet; that arrives with the capture's own record
//! (T6/drain). So the line names what it knows and invents nothing.
//!
//! Nothing is drawn for an entry with no provenance beyond its own words: the
//! operator's message, an answer, a notice and a stop are all of themselves.

use ratatui::text::Line;

use super::transcript::TranscriptFacts;
use super::turn_rows::{GUTTER, plain};
use crate::front::tui::theme::Token;
use crate::front::view::{CommandState, Entry, HelperState, elapsed_text};

/// The provenance line for `entry`, or nothing when the entry has none.
pub(super) fn row(entry: &Entry, facts: &TranscriptFacts, content: usize) -> Option<Line<'static>> {
    let words = match entry {
        Entry::Activity(activity) => {
            let steps = steps_text(activity.steps.len());
            match activity.failed {
                true => format!("{steps} · one failed"),
                false => steps,
            }
        }
        // The one block whose provenance is a result's: the connection that ran
        // it and the rows it handed the front, or the refusal that kept them.
        Entry::ResultPreview(preview) => match preview.refusal {
            Some(_) => format!("{} · no rows held for the model", preview.connection),
            None => format!(
                "{} · {}",
                preview.connection,
                rows_text(preview.result.row_count)
            ),
        },
        Entry::Command(command) => state_words(&command.state),
        Entry::Edit(edit) => format!("{} · +{} −{}", edit.path, edit.added(), edit.removed()),
        Entry::Helper(helper) => format!(
            "{} · {} · {}",
            helper.id,
            steps_text(helper.steps),
            helper_words(helper.state)
        ),
        _ => return None,
    };
    Some(plain(GUTTER, &words, Token::Dim, facts, content))
}

/// A command's own ending, as the inspector says it.
fn state_words(state: &CommandState) -> String {
    match state {
        CommandState::Finished { elapsed } => format!("finished · {}", elapsed_text(*elapsed)),
        CommandState::Failed { elapsed, .. } => format!("failed · {}", elapsed_text(*elapsed)),
    }
}

/// A helper's own state, as the inspector says it.
fn helper_words(state: HelperState) -> &'static str {
    match state {
        HelperState::Running => "running",
        HelperState::Done => "done",
        HelperState::Failed => "failed",
    }
}

/// How many steps a group holds, in the number the count takes.
fn steps_text(steps: usize) -> String {
    match steps {
        1 => "1 step".to_owned(),
        steps => format!("{steps} steps"),
    }
}

/// How many rows a result holds, in the number the count takes.
fn rows_text(rows: usize) -> String {
    match rows {
        1 => "1 row".to_owned(),
        rows => format!("{rows} rows"),
    }
}
