//! Test support for the help sheet: the states a session can be in, and the
//! sheet read back as the words a terminal shows.

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::composer::Row;
use super::help::{self, Context};
use super::help_keys::{Chord, When};
use crate::front::tui::model::Model;
use crate::front::tui::view::cards_fixture::shell_question;
use crate::front::tui::view::jobs_fixture;
use crate::front::{Entry, UserMessage};

/// A press with no modifier held.
pub(super) const fn plain(code: KeyCode) -> Chord {
    Chord {
        code,
        modifiers: KeyModifiers::NONE,
    }
}

/// The context an idle surface is in: the glyph set, nothing else.
pub(super) fn idle() -> Context {
    Context::default()
}

/// The sheet's rows at `width` x `height`, as their text.
pub(super) fn words(context: Context, width: usize, height: usize) -> Vec<String> {
    help::rows(context, width, height)
        .iter()
        .map(inside)
        .collect()
}

/// One row's text, without its border cells or the padding before the right
/// edge.
fn inside(row: &Row) -> String {
    let text = row.text();
    let inner: String = text.chars().skip(2).collect();
    let kept = inner.chars().count().saturating_sub(2);
    let mut trimmed: String = inner.chars().take(kept).collect();
    trimmed.truncate(trimmed.trim_end().len());
    trimmed
}

/// The model a row valid in `when` is pressed against: the state the row says
/// it belongs to, and nothing else.
pub(super) fn model_in(when: When) -> Model {
    let mut model = Model::default();
    match when {
        // A card is open with its panel holding the keyboard, and its arrows
        // have pointed at an answer: the one state in which every key the
        // approvals group lists — Enter among them — answers the call.
        When::Card | When::CardGrant => {
            model.show_card(shell_question());
            model.move_choice(1);
        }
        When::Composer => {}
        When::Queued => {
            model.start_turn();
            model.queue("and then the second one".to_owned());
        }
        When::Busy => model.start_turn(),
        When::Search => model.open_search(),
        When::Cursor => {
            model.start_turn();
            model
                .transcript
                .current()
                .push(Entry::UserMessage(UserMessage {
                    text: String::from("hello"),
                }));
            model.move_focus(true);
        }
        When::Jobs => model.open_jobs(),
        When::JobsConfirm => {
            model.note_jobs(crate::front::tui::jobs::JobReport::Listed(
                jobs_fixture::jobs(),
            ));
            model.open_jobs();
            if let Some(drawer) = model.jobs_mut() {
                drawer.ask_cancel();
            }
        }
    }
    model
}
