//! The frame's grid and the facts its status row is painted from: the one place
//! `02` §1's regions are measured, kept apart from the panes that fill them.

use ratatui::layout::{Constraint, Layout, Rect};

use super::model::Model;
use super::run_draw::Facts;
use super::run_draw_card;
use super::view::{ActivityState, StatusFacts, rows as composer_rows};

/// column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Regions {
    pub(crate) transcript: Rect,
    pub(crate) status: Rect,
    /// The docked card's rows, when a call is open.
    pub(crate) card: Option<Rect>,
    /// The toast's one row, when the surface is saying something.
    pub(crate) toast: Option<Rect>,
    pub(crate) composer: Rect,
}

/// The grid's regions for `area`, in the order the panes sit.
pub(crate) fn regions(model: &Model, facts: &Facts, area: Rect) -> Regions {
    let column = column(area);
    let height = column.height as usize;
    let width = column.width as usize;
    // The composer keeps its box and its hint row whatever else is on screen;
    // the status line is one row and never folds.
    let composer = composer_rows(model, width, facts.charset)
        .len()
        .min(height.saturating_sub(1))
        .max(1);
    let spare = height.saturating_sub(1 + composer);
    let card = run_draw_card::rows(model, column.height, facts.host_unsandboxed).min(spare);
    let toast = usize::from(model.toast().is_some() && spare > card);
    let [transcript, status, docked, said, composer] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(card as u16),
        Constraint::Length(toast as u16),
        Constraint::Length(composer as u16),
    ])
    .areas(column);
    Regions {
        transcript,
        status,
        card: (card > 0).then_some(docked),
        toast: (toast > 0).then_some(said),
        composer,
    }
}

/// `02` §1's content column: one cell of gutter on each side.
fn column(area: Rect) -> Rect {
    let [_, middle, _] = Layout::horizontal([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);
    middle
}

/// The status line's facts: the model's mode and database, the launch's model,
/// and what the surface is doing right now. Nothing conditional beyond the
/// sharing caveat lives here: the unsandboxed warning belongs to the approval
/// card, where the decision is made.
pub(crate) fn status_facts(model: &Model, facts: &Facts) -> StatusFacts {
    StatusFacts {
        mode: model.mode(),
        databases: model
            .database_name()
            .map(|name| vec![name.to_owned()])
            .unwrap_or_default(),
        model: facts.model.clone(),
        sharing: facts.sharing,
        activity: activity(model, facts),
        scheme: facts.scheme,
        charset: facts.charset,
    }
}

/// What the status line says the surface is doing: a held line outranks a
/// running turn, because the held line is what the next Enter acts on.
fn activity(model: &Model, facts: &Facts) -> ActivityState {
    if model.queued().is_some() {
        ActivityState::Queued { count: 1 }
    } else if model.is_busy() {
        ActivityState::Working {
            seconds: facts.working.map_or(0, |working| working.as_secs()),
        }
    } else {
        ActivityState::Idle
    }
}
