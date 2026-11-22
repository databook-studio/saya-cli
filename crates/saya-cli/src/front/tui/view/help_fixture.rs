//! Test support for the help sheet: the states a session can be in, the presses
//! the sheet has to cover, and the sheet read back as the words a terminal
//! shows.

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::composer::Row;
use super::help::{self, Context};
use super::help_keys::{Chord, Shortcut};
use crate::front::tui::model::Model;
use crate::front::tui::view::cards_fixture::shell_question;

/// A press with no modifier held.
pub(super) const fn plain(code: KeyCode) -> Chord {
    Chord {
        code,
        modifiers: KeyModifiers::NONE,
    }
}

/// Ctrl+G, the key that drops the line held behind a running turn (`02` §7).
pub(super) const fn ctrl_g() -> Chord {
    Chord {
        code: KeyCode::Char('g'),
        modifiers: KeyModifiers::CONTROL,
    }
}

/// Ctrl+K, the chord that opens `02` §11's command palette.
pub(super) const fn ctrl_k() -> Chord {
    Chord {
        code: KeyCode::Char('k'),
        modifiers: KeyModifiers::CONTROL,
    }
}

/// The presses the card answers, and the ones it must not: what the sheet is
/// asked about in either direction.
pub(super) fn probes() -> Vec<Chord> {
    vec![
        plain(KeyCode::Char('y')),
        plain(KeyCode::Char('a')),
        plain(KeyCode::Char('s')),
        plain(KeyCode::Char('n')),
        plain(KeyCode::Char('d')),
        plain(KeyCode::Esc),
        plain(KeyCode::Char('v')),
        plain(KeyCode::Up),
        plain(KeyCode::Down),
        plain(KeyCode::PageUp),
        plain(KeyCode::PageDown),
        plain(KeyCode::Enter),
        plain(KeyCode::Backspace),
        ctrl_g(),
    ]
}

/// The context an idle surface is in: no card, nothing running, nothing held.
pub(super) fn idle() -> Context {
    Context::default()
}

/// The context a running turn is in, with nothing held behind it.
pub(super) fn busy() -> Context {
    Context {
        busy: true,
        ..Context::default()
    }
}

/// The context a running turn with one line held behind it is in.
pub(super) fn queued() -> Context {
    Context {
        busy: true,
        queued: true,
        ..Context::default()
    }
}

/// The context a card offering a grant puts the surface in.
pub(super) fn with_card() -> Context {
    Context {
        card: true,
        grant: true,
        busy: true,
        ..Context::default()
    }
}

/// The context a card offering no token puts the surface in.
pub(super) fn with_ungranted_card() -> Context {
    Context {
        card: true,
        grant: false,
        busy: true,
        ..Context::default()
    }
}

/// Every press the sheet lists in `context`.
pub(super) fn chords(context: &Context) -> Vec<Chord> {
    help::listed(context)
        .iter()
        .flat_map(|shortcut| shortcut.chords.iter().copied())
        .collect()
}

/// The row the sheet files `chord` under in `context`, if any.
pub(super) fn row(context: &Context, chord: Chord) -> Option<&'static Shortcut> {
    help::listed(context)
        .into_iter()
        .find(|shortcut| shortcut.chords.contains(&chord))
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
pub(super) fn model_in(when: super::help_keys::When) -> Model {
    use super::help_keys::When;

    let mut model = Model::default();
    match when {
        When::Card | When::CardGrant => model.show_card(shell_question()),
        When::Composer => {}
        When::Queued => {
            model.start_turn();
            model.queue("and then the second one".to_owned());
        }
        When::Busy => model.start_turn(),
    }
    model
}
