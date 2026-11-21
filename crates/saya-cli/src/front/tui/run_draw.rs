//! One frame: the panes T4-T13 built, laid out as `02` §1's grid.
//!
//! Nothing here decides what a pane says — each of them is a pure function of
//! the facts it is handed — and nothing here reads a clock or the terminal: the
//! runtime says what the frame is painted with ([`Facts`]) and hands over the
//! area, and this module paints status line, transcript, docked card and
//! composer, bottom-anchored, so the newest thing an operator is looking at is
//! the one that stays on screen when the screen is short.
//!
//! The grid (`02` §1), from the bottom up: the composer's box and its hint row;
//! the docked card, when a call is open; the status line, one row, never
//! wrapped; and the transcript above it, keeping whatever is left. Every pane
//! is painted one cell inside the terminal's edges, so a frame never writes into
//! the last column and makes the terminal wrap by itself.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::model::Model;
use super::run_draw_card;
use super::theme::{Charset, Scheme, style};
use super::view::{
    ActivityState, Database, EmptyFacts, StatusFacts, TranscriptFacts, empty_rows,
    rows as composer_rows, status_line, transcript_rows,
};

/// What a frame is painted with, besides the model: the token column and glyph
/// set the session was launched under, how long the running turn has run, and
/// the two launch facts the model does not hold — whether a provider answers,
/// and the workspace root as the launch abbreviates it (`~/shop`). The empty
/// transcript is the one pane that needs them (`02` §9); the rest read the
/// model.
#[derive(Debug, Clone)]
pub(crate) struct Facts {
    pub(crate) scheme: Scheme,
    pub(crate) charset: Charset,
    /// How long the turn in flight has run, for the status line's clock.
    pub(crate) working: Option<Duration>,
    /// Whether a provider is configured to answer at all.
    pub(crate) provider: bool,
    /// The workspace root, abbreviated, or `None` when no project is bound.
    pub(crate) workspace: Option<String>,
}

impl Default for Facts {
    fn default() -> Self {
        Self {
            scheme: Scheme::Dark,
            charset: Charset::Utf8,
            working: None,
            // A run that opened this surface was launched with a provider: the
            // first-run screen is for a launch that has none, and the launch
            // states that itself.
            provider: true,
            workspace: None,
        }
    }
}

/// The four regions of `02` §1's grid, top to bottom, inside the content
/// column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Regions {
    pub(crate) transcript: Rect,
    pub(crate) status: Rect,
    /// The docked card's rows, when a call is open.
    pub(crate) card: Option<Rect>,
    pub(crate) composer: Rect,
}

/// Paints one whole frame.
pub(crate) fn frame(model: &Model, facts: &Facts, frame: &mut Frame) {
    let regions = regions(model, facts, frame.area());
    let transcript = transcript(model, facts, regions.transcript);
    paint(frame, regions.transcript, transcript);
    let width = regions.status.width as usize;
    paint(
        frame,
        regions.status,
        vec![status_line(&status_facts(model, facts), width)],
    );
    if let (Some(area), true) = (regions.card, model.card().is_some()) {
        paint(frame, area, run_draw_card::card(model, facts, area.height));
    }
    let width = regions.composer.width as usize;
    paint(frame, regions.composer, composer(model, facts, width));
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
    let card = run_draw_card::rows(model, column.height).min(spare);
    let [transcript, status, docked, composer] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(card as u16),
        Constraint::Length(composer as u16),
    ])
    .areas(column);
    Regions {
        transcript,
        status,
        card: (card > 0).then_some(docked),
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

/// Paints one region's rows, clipping anything longer than the region.
fn paint(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    frame.render_widget(Paragraph::new(lines), area);
}

/// The transcript: the conversation's rows, or the empty state when there is no
/// conversation yet. Both panes keep the newest rows on screen.
fn transcript(model: &Model, facts: &Facts, area: Rect) -> Vec<Line<'static>> {
    let width = area.width as usize;
    let height = area.height as usize;
    if model.turns().is_empty() {
        return empty_rows(&empty_facts(model, facts), width, height);
    }
    transcript_rows(
        model.turns(),
        &TranscriptFacts {
            workspace: facts.workspace.clone(),
            scheme: facts.scheme,
            charset: facts.charset,
            ..TranscriptFacts::default()
        },
        width,
        height,
    )
}

/// The facts the empty transcript is painted from: what the model knows about
/// the database, and what the launch knows about the provider and the project.
fn empty_facts(model: &Model, facts: &Facts) -> EmptyFacts {
    EmptyFacts {
        provider: facts.provider,
        database: model.database_name().map(|name| Database {
            name: name.to_owned(),
            ..Database::default()
        }),
        workspace: facts.workspace.clone(),
        scheme: facts.scheme,
        charset: facts.charset,
        ..EmptyFacts::default()
    }
}

/// The status line's facts: the model's mode, database, queue and toast, and
/// what the surface is doing right now.
fn status_facts(model: &Model, facts: &Facts) -> StatusFacts {
    StatusFacts {
        mode: model.mode(),
        databases: model
            .database_name()
            .map(|name| vec![name.to_owned()])
            .unwrap_or_default(),
        activity: activity(model, facts),
        toast: model.toast().map(str::to_owned),
        scheme: facts.scheme,
        charset: facts.charset,
        ..StatusFacts::default()
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

/// The composer's box: its rows' runs, each in the token the pane gave it.
fn composer(model: &Model, facts: &Facts, width: usize) -> Vec<Line<'static>> {
    composer_rows(model, width, facts.charset)
        .into_iter()
        .map(|row| {
            Line::from(
                row.runs
                    .into_iter()
                    .map(|run| Span::styled(run.text, style(run.token, facts.scheme)))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}
