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
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::model::Model;
use super::run_draw_card;
use super::run_draw_layout::{regions, status_facts};
use super::run_draw_overlay;
use super::theme::{Charset, Scheme, ascii_line, style};
use super::view::{
    Database, EmptyFacts, Row, TranscriptFacts, empty_rows, rows as composer_rows, status_line,
    toast_row, transcript_rows,
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
    /// Whether the session bound no workspace and has not answered the trust
    /// question yet: the runtime opens the question on it (`02` §9.4).
    pub(crate) untrusted: bool,
    /// The model's short id, for the status line's segment.
    pub(crate) model: String,
    /// Whether host commands run outside the sandbox: the approval card states
    /// it where the decision is made.
    pub(crate) host_unsandboxed: bool,
    /// Whether query data leaves the machine: the status line's one caveat.
    pub(crate) sharing: bool,
    /// The workspace's own top-level names, for the welcome screen's
    /// suggestions. Every one exists; an empty list offers neutral examples.
    pub(crate) entries: Vec<String>,
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
            untrusted: false,
            model: String::new(),
            host_unsandboxed: false,
            sharing: false,
            entries: Vec::new(),
        }
    }
}

/// Paints one whole frame.
pub(crate) fn frame(model: &Model, facts: &Facts, frame: &mut Frame) {
    let regions = regions(model, facts, frame.area());
    let transcript = transcript(model, facts, regions.transcript);
    paint(frame, regions.transcript, transcript, facts.charset);
    // A pane that is open takes the transcript's rows: nothing else on the
    // frame gives way, so the operator keeps the mode in force and their draft.
    if let Some((area, rows)) =
        run_draw_overlay::overlay(model, facts, regions.transcript, frame.area().width)
    {
        paint(frame, area, rows, facts.charset);
    }
    let width = regions.status.width as usize;
    paint(
        frame,
        regions.status,
        vec![status_line(&status_facts(model, facts), width)],
        facts.charset,
    );
    if let (Some(area), true) = (regions.card, model.card().is_some()) {
        // The body's cap is the *screen's* budget, so the card is laid out from
        // the frame it is painted in — the region is what the cap already left.
        paint(
            frame,
            area,
            run_draw_card::card(model, facts, frame.area().height, area.width),
            facts.charset,
        );
    }
    if let (Some(area), Some(toast)) = (regions.toast, model.toast()) {
        let width = area.width as usize;
        paint(
            frame,
            area,
            lines(vec![toast_row(toast, width, facts.charset)], facts.scheme),
            facts.charset,
        );
    }
    let width = regions.composer.width as usize;
    paint(
        frame,
        regions.composer,
        composer(model, facts, width),
        facts.charset,
    );
}

/// Paints one region's rows, clipping anything longer than the region. Under
/// `--ascii` every row is turned seven-bit first, so a glyph no pane owns — a
/// table's border, a copy `·`, a `…` — cannot reach a terminal that cannot
/// draw it.
fn paint(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>, charset: Charset) {
    let lines = match charset {
        Charset::Utf8 => lines,
        Charset::Ascii => lines.into_iter().map(ascii_line).collect(),
    };
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
            focus: model.focus(),
            inspector: model.inspector(),
            detail: model.detail(),
            spinner: model.spinner(),
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
        suggestions: super::view::empty_suggestions::from_entries(&facts.entries),
        scheme: facts.scheme,
        charset: facts.charset,
        ..EmptyFacts::default()
    }
}

/// The composer's box: its rows' runs, each in the token the pane gave it.
fn composer(model: &Model, facts: &Facts, width: usize) -> Vec<Line<'static>> {
    lines(composer_rows(model, width, facts.charset), facts.scheme)
}

/// Painted rows: every run in the token the pane gave it, so the panes above
/// resolve light, dark and no-colour without knowing what the words mean.
pub(crate) fn lines(rows: Vec<Row>, scheme: Scheme) -> Vec<Line<'static>> {
    rows.into_iter()
        .map(|row| {
            Line::from(
                row.runs
                    .into_iter()
                    .map(|run| Span::styled(run.text, style(run.token, scheme)))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}
