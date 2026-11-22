//! The accessibility pass's pane set: each pane the crate renders, as a whole
//! pane's words, so a test can compare the colour-off reading with the coloured
//! one without a terminal.
//!
//! The values a pane is painted from are [`super::a11y_samples`]'s; this module
//! names the panes and turns each one's rows into text. Every sample states the
//! design's own case (`02` §2-§11), so a pane reworded into agreeing with itself
//! still reads.

use ratatui::text::Line;

use super::a11y_samples;
use super::theme::{Charset, Scheme};
use super::view::{
    empty_rows, help_rows, jobs_rows, mode_picker_rows, palette_rows, result_rows,
    rows as composer_rows, sessions_rows, status_line, transcript_rows,
};

/// One pane of the surface the crate renders, by the name a failure mentions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane {
    Status,
    Transcript,
    Composer,
    Card,
    Palette,
    Help,
    Sessions,
    ModePicker,
    Jobs,
    Result,
    Empty,
    Blocks,
}

/// Every pane of `02` §2-§11 the crate paints.
pub(crate) const ALL: [Pane; 12] = [
    Pane::Status,
    Pane::Transcript,
    Pane::Composer,
    Pane::Card,
    Pane::Palette,
    Pane::Help,
    Pane::Sessions,
    Pane::ModePicker,
    Pane::Jobs,
    Pane::Result,
    Pane::Empty,
    Pane::Blocks,
];

impl Pane {
    /// The pane's name, as a failure states it.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Status => "status line",
            Self::Transcript => "transcript",
            Self::Composer => "composer",
            Self::Card => "docked card",
            Self::Palette => "command palette",
            Self::Help => "help sheet",
            Self::Sessions => "sessions picker",
            Self::ModePicker => "mode picker",
            Self::Jobs => "jobs drawer",
            Self::Result => "result table",
            Self::Empty => "empty state",
            Self::Blocks => "transcript blocks",
        }
    }
}

/// The pane's whole text, row by row, in `scheme` and `charset`.
pub(crate) fn render(pane: Pane, scheme: Scheme, charset: Charset) -> Vec<String> {
    let facts = a11y_samples::transcript_facts(scheme, charset);
    match pane {
        Pane::Status => lines(&[status_line(
            &a11y_samples::status_facts(scheme, charset),
            100,
        )]),
        Pane::Transcript => lines(&transcript_rows(
            &[a11y_samples::conversation()],
            &facts,
            100,
            40,
        )),
        Pane::Composer => row_texts(&composer_rows(&a11y_samples::draft(), 100, charset)),
        Pane::Card => a11y_samples::card_rows(),
        Pane::Palette => row_texts(&palette_rows(&a11y_samples::palette(charset), 100, 20)),
        Pane::Help => row_texts(&help_rows(a11y_samples::help_context(charset), 100, 20)),
        Pane::Sessions => row_texts(&sessions_rows(&a11y_samples::picker(charset), 100, 20)),
        Pane::ModePicker => row_texts(&mode_picker_rows(
            &a11y_samples::mode_picker(),
            100,
            10,
            charset,
        )),
        Pane::Jobs => row_texts(&jobs_rows(&a11y_samples::drawer(charset), 100, 6, charset)),
        Pane::Result => lines(&result_rows(&a11y_samples::preview(), &facts, 100)),
        Pane::Empty => lines(&empty_rows(
            &a11y_samples::empty_facts(scheme, charset),
            100,
            20,
        )),
        Pane::Blocks => lines(&transcript_rows(&[a11y_samples::blocks()], &facts, 100, 40)),
    }
}

/// Painted rows as their text.
fn row_texts(rows: &[super::view::Row]) -> Vec<String> {
    rows.iter().map(super::view::Row::text).collect()
}

/// Painted lines as their text, styles dropped.
pub(crate) fn lines(rows: &[Line<'_>]) -> Vec<String> {
    rows.iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect()
}

/// One pane's whole text as one string, for a `contains` check.
pub(crate) fn words(pane: Pane, scheme: Scheme, charset: Charset) -> String {
    render(pane, scheme, charset).join("\n")
}
