//! Transcript rendering: label rows, role marks, and the scrollbar.

use super::super::theme::{accent, kind_style, label_style};
use super::markdown::markdown_spans_fenced;
use crate::interactive::tui::transcript::BlockKind;
use crate::interactive::tui::types::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span, Text},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};

/// Spinner frames shown while an agent request is streaming.
pub(in crate::interactive::tui) const SPINNER: [&str; 10] =
    ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The glyph for a kind that has no label word, or `None` when the kind is
/// introduced by a label row instead.
///
/// This is the interim half of the visual grammar. `YOU`/`SAYA`/`ACTIVITY`/
/// `RESULT` are stated in words; `Error`, `System` and `Thinking` have no
/// agreed word yet, so they keep the glyph that distinguished them before —
/// two columns wide, exactly like the body indent, so nothing shifts.
///
/// The gate for this phase is that nothing essential relies on hue alone. A
/// failure whose only difference from prose is a red foreground fails it,
/// which is why these glyphs may not be removed until the phase that names
/// them (failure headline: Phase 7; System content: undecided) lands.
pub(in crate::interactive::tui) fn unlabelled_glyph(kind: BlockKind) -> Option<&'static str> {
    match kind {
        BlockKind::Error => Some("\u{2717} "),
        BlockKind::System => Some("\u{b7} "),
        BlockKind::Thinking => Some("\u{2248} "),
        // These are introduced by a label row; a glyph as well would be
        // saying the same thing twice.
        BlockKind::User | BlockKind::Assistant | BlockKind::Tool | BlockKind::Table => None,
    }
}

/// The draft wording for a streaming answer's label row: `SAYA (draft)` — a
/// plain word, never a spinner or animation. It says what the block is (the
/// answer, still arriving) without claiming progress, a percentage, or a
/// motive; it vanishes on `Done` because it is read from `request.stream` at
/// paint time, and it never touches `block.text` so copy is unaffected.
pub(super) const DRAFT_LABEL: &str = "SAYA (draft)";

/// Renders the visible, soft-wrapped transcript lines — a label word at the
/// left margin introducing each turn, body rows indented beneath it — with
/// per-kind styling, plus a scrollbar when the content overflows.
pub(in crate::interactive::tui) fn draw_transcript(frame: &mut Frame<'_>, app: &App, area: Rect) {
    // Reserve two columns on the left for the role rail; wrap text to the rest.
    let text_width = area.width.saturating_sub(2);
    // Store the WRAP width (not the pane width) so key-driven scrolling clamps consistently.
    app.viewport.set((text_width, area.height));
    let width = text_width as usize;
    let height = area.height as usize;

    let mut lines: Vec<Line> = Vec::new();
    // ``` fence state persists across the consecutive lines of one assistant
    // block; any other role ends it.
    let mut fence = false;
    // Every row `wide_view` returns paints: a label row paints its word at
    // the left margin, a body row paints indented by 2 spaces, and an empty
    // row stays blank. The 2-space indent keeps the 2-column rail reserve,
    // so the wrap width and the scroll clamp are unchanged.
    for (i, row) in app
        .transcript
        .wide_view(width, height, &app.wide_table)
        .into_iter()
        .enumerate()
    {
        if row.is_label {
            // An assistant label in the live chapter while the request still
            // streams is a draft: read at paint time from the live state, so
            // `Done` clears it with no extra plumbing and `block.text` —
            // what copy sees — is untouched.
            let draft = row.kind == BlockKind::Assistant
                && app.request.stream.is_some()
                && is_live_assistant_label(&app.transcript, row.text.as_str(), i, width);
            let text = if draft {
                DRAFT_LABEL
            } else {
                row.text.as_str()
            };
            lines.push(Line::from(Span::styled(text.to_string(), label_style())));
            continue;
        }
        let kind = row.kind;
        let text = row.text;
        if text.is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        if kind == BlockKind::Assistant {
            // The body indents beneath its label; markdown still keys off
            // the text (headings, bullets, fences) and the indent paints as
            // a plain prefix ahead of the shaped spans.
            let mut spans = vec![Span::raw("  ")];
            spans.extend(markdown_spans_fenced(&text, &mut fence));
            lines.push(Line::from(spans));
            continue;
        }
        fence = false;
        // A kind with no label keeps its glyph in the indent. `Error`,
        // `System` and `Thinking` have no label word yet (`rows::label`
        // returns `None`), and the glyph was their only distinction that
        // survives without colour. Dropping it would leave a failure
        // reading as ordinary prose and fold a memory receipt into the
        // answer above it — so it stays until a label replaces it, which
        // is the plan's rule: do not hide information until its
        // replacement is visible.
        let prefix = unlabelled_glyph(kind).unwrap_or("  ");
        lines.push(Line::from(Span::styled(
            format!("{prefix}{text}"),
            kind_style(kind),
        )));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);

    let (total, first_visible) = app.transcript.scroll_metrics(width, height);
    if total > height {
        let mut state = ScrollbarState::new(total).position(first_visible);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::default().fg(accent())),
            area,
            &mut state,
        );
    }
}

/// Whether the label row painted at `painted_idx` is the live chapter's
/// assistant label — the only one that may read as a draft. The painted rows
/// carry no chapter, so count back from the window: the live chapter opens at
/// the last `YOU` row across the whole transcript, and only an assistant
/// label after it is the live answer. Earlier chapters — resumed or folded —
/// keep exactly today's `SAYA`.
fn is_live_assistant_label(
    transcript: &crate::interactive::tui::transcript::Transcript,
    text: &str,
    painted_idx: usize,
    width: usize,
) -> bool {
    use crate::interactive::tui::transcript::rows::label;
    if text != label(BlockKind::Assistant).unwrap_or("SAYA") {
        return false;
    }
    live_assistant_idx(transcript, width).is_some_and(|live| {
        let (_, first_visible) = transcript.scroll_metrics(width, usize::MAX);
        live == first_visible.saturating_add(painted_idx)
    })
}

/// The full-transcript index of the live chapter's assistant label, if the
/// live chapter has one on screen: the last assistant label after the last
/// user label. `None` with no user turn yet (welcome text, resumed prose) or
/// when the live chapter has streamed no answer text so far.
fn live_assistant_idx(
    transcript: &crate::interactive::tui::transcript::Transcript,
    width: usize,
) -> Option<usize> {
    use crate::interactive::tui::transcript::BlockKind;
    let full = transcript.wrapped(width);
    let open = full
        .iter()
        .rposition(|r| r.is_label && r.kind == BlockKind::User)?;
    full.iter()
        .rposition(|r| r.is_label && r.kind == BlockKind::Assistant)
        .filter(|&idx| {
            open < idx
                && !full[idx + 1..]
                    .iter()
                    .any(|r| r.is_label && r.kind == BlockKind::User)
        })
}
