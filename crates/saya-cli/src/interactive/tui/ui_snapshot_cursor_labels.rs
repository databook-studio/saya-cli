use super::super::stream_events::apply_event;
use super::super::transcript::BlockKind;
/// Composed-screen behaviour snapshots (input cursor mapping and transcript label rows), moved byte-identical from the hub.
/// Plain assertions only — the five `insta` snapshots stay in the hub.
use super::support::{empty_app, empty_app_with_text, fixed_status, render_buffer, render_cursor};
use saya_agent::AgentEvent;

// --- Input box: wrap-aware cursor mapping through the real render path. -------
//
// These are the regression tests for the input-box truncation defect
// (databook-studio/saya-cli#57). A question longer than the terminal width used
// to render as one truncated row with the cursor pinned against the right
// border. The fix wraps the line and maps the logical cursor to a visual
// (row, col) against the inner width. Each test draws through the real
// `ui::draw` and reads the cursor position back, so a mapping regression is
// caught here rather than by eye.
//
// Layout at 40×24: the input box sits at the bottom. With a 1-row input the
// box is 3 rows tall (1 text + 2 border), so its inner area is row 21, cols
// 1..39 (inner width 38). A longer input grows the box upward.

/// The inner width of the input box at a 40-column terminal: the box spans
/// cols 0..40 with a rounded border, leaving 38 usable columns.
const INPUT_INNER_WIDTH: usize = 38;

/// A line exactly the inner width keeps the cursor on the single row at the
/// column just past the text — no wrap, no spurious extra row, cursor not
/// pinned against the border. (Spec test list item 1, rendered.)
#[test]
fn input_cursor_stays_on_one_row_for_an_exact_width_line() {
    let mut app = empty_app();
    // 38 chars: exactly the inner width at 40 columns.
    app.input.set_text("a".repeat(INPUT_INNER_WIDTH));
    let (x, _y) = render_cursor(&app, &fixed_status(), 40, 24);
    // inner.x = 1; cursor col = 38 -> screen x = 1 + 38 = 39 (last inner cell).
    assert_eq!(
        x,
        1 + INPUT_INNER_WIDTH as u16,
        "exact-width line: cursor at the last inner column, not past the border"
    );
}

/// A line one character over wraps to two visual rows and the cursor lands on
/// the second row, at column 1 (after the wrapped char). This is the
/// make-or-break case: the old path pinned the cursor against the right
/// border; the wrap-aware path puts it on row 2. (Spec test list item 2,
/// rendered — deliverable 1.)
#[test]
fn input_cursor_wraps_to_second_row_when_line_exceeds_width() {
    let mut app = empty_app();
    // 39 chars: one over the 38-col inner width -> 2 visual rows, cursor at end.
    app.input.set_text("a".repeat(INPUT_INNER_WIDTH + 1));
    let (x, y) = render_cursor(&app, &fixed_status(), 40, 24);
    // inner.x = 1; the cursor is on the second visual row at col 1 (after the
    // single wrapped 'a') -> screen x = 2. The old path put x at the border
    // (40) because it used the logical column.
    assert_eq!(
        x, 2,
        "one char over: cursor column is 2 (inner.x=1 + wrapped col=1), not the border"
    );

    // The box grew to hold the wrap: a 1-row input claims 1 visual row, a line
    // one char over claims 2. Asserted through `input_rows` directly so it
    // does not depend on the splash art that fills the empty transcript pane.
    let two = empty_app_with_text(&"a".repeat(INPUT_INNER_WIDTH + 1));
    let one = empty_app_with_text(&"a".repeat(INPUT_INNER_WIDTH));
    assert_eq!(
        two.input_rows(40),
        2,
        "a wrapped line grows the box to 2 visual rows"
    );
    assert_eq!(
        one.input_rows(40),
        1,
        "an exact-width line keeps the box at 1 visual row"
    );

    // And the wrap is visible: the buffer shows two content rows of 'a's
    // inside the box (the buffer view quotes each row, so match the inner
    // border+content). This guards against a regression to one truncated row.
    let buffer_two = render_buffer(&two, &fixed_status(), 40, 24);
    let wrapped_rows = buffer_two.lines().filter(|l| l.contains("│a")).count();
    assert!(
        wrapped_rows >= 2,
        "the wrapped line shows two content rows, not one truncated row:\n{buffer_two}"
    );
    let _ = (y, app);
}

/// Cursor at position 0 of an empty buffer: the placeholder path still parks
/// the cursor at the inner top-left. (Spec test list item 4, rendered.)
#[test]
fn input_empty_buffer_cursor_at_origin() {
    let app = empty_app();
    let (x, y) = render_cursor(&app, &fixed_status(), 40, 24);
    // inner.x = 1, inner.y = top inner row of the 3-row box.
    assert_eq!(x, 1, "empty buffer: cursor at inner left");
    // The box is the last 3 rows (21..24); inner top is row 22.
    assert_eq!(y, 22, "empty buffer: cursor at inner top row");
}

// --- Fieldnotes phase 2, packet 2B-3: label rows paint. --------------------
//
// RED packet: label rows exist in `lines()` (measured) but `view()` /
// `wide_view()` elide them before the window slice and the painters skip
// them, so measured height exceeds painted height. These tests fail until
// the elision is removed and the label word paints on its own row.

/// A long user request wrapping to several rows shows `YOU` exactly once:
/// the label introduces the turn, the body rows carry no label.
#[test]
fn continuation_lines_carry_no_label() {
    let mut app = empty_app();
    app.transcript.push(
        BlockKind::User,
        "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu",
    );
    // Narrow enough that the body wraps to several rows.
    let buffer = render_buffer(&app, &fixed_status(), 40, 24);
    let you_rows = buffer
        .lines()
        .filter(|line| line.trim_start_matches(['"', ' ']).starts_with("YOU"))
        .count();
    assert_eq!(
        you_rows, 1,
        "the user turn introduces exactly one YOU label row:\n{buffer}"
    );
    assert!(
        !buffer.contains("❯ "),
        "no glyph rail paints anymore:\n{buffer}"
    );
}

/// The run-panel episode introduces its turn with the same word the
/// transcript uses: the shared `transcript::rows::label` map, one source.
#[test]
fn episode_first_line_carries_the_shared_label() {
    use super::super::run_panel::RunPanel;
    use super::super::run_worker::RunWorker;

    let expected = super::super::transcript::rows::label(BlockKind::Assistant);
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "count the orders");
    let (tx, rx) = super::super::run_panel::test_channels();
    let mut panel = RunPanel::new(
        RunWorker {
            rx,
            cancel: saya_agent::CancellationToken::new(),
        },
        "r-label".into(),
        "survey".into(),
    );
    let _ = tx;
    apply_event(
        &mut panel.episode,
        AgentEvent::assistant_text("Step one profiled the tables."),
        false,
    );
    app.run_panel = Some(panel);
    let buffer = render_buffer(&app, &fixed_status(), 100, 30);
    let word = expected.expect("user turns have a label");
    assert!(
        buffer.contains(word),
        "the episode paints the shared label {word:?}:\n{buffer}"
    );
}

/// An error row carries no introducing label (no `ERROR` label exists —
/// inventing one belongs to a later phase), so without colour it paints
/// exactly like ordinary prose: indented, no glyph. This pins the known
/// gap: stripped of style, a failure is text-identical to a plain body row.
#[test]
fn a_failure_is_not_distinguished_by_colour_alone() {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "hi");
    app.transcript.push(BlockKind::Error, "boom");
    // `render_buffer` strips colour, so anything this sees is hue-independent
    // by construction. The phase gate is that nothing essential relies on hue
    // alone; a failure that paints as plain indented prose would fail it.
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("✗ boom"),
        "a failure keeps a mark that survives with colour stripped:\n{buffer}"
    );
    assert!(
        !buffer.contains("ERROR"),
        "and no ERROR label is invented to provide it — the failure headline \
         is Phase 7's work:\n{buffer}"
    );
}

/// `System` content sits between turns. Without a mark of its own it is
/// indented exactly like assistant prose and reads as part of the answer
/// above it, which misattributes it — the opposite of the phase's
/// who-said-what goal.
#[test]
fn system_content_is_not_absorbed_into_the_answer_above_it() {
    let mut app = empty_app();
    app.transcript
        .push(BlockKind::Assistant, "here is the answer");
    app.transcript
        .push(BlockKind::System, "memory supplied · 1 claim");
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("  here is the answer"),
        "the answer body indents under SAYA:\n{buffer}"
    );
    assert!(
        buffer.contains("· memory supplied"),
        "the receipt keeps a mark distinguishing it from that answer:\n{buffer}"
    );
}

/// `total_lines(w)` equals the row count `wide_view` returns for a tall
/// enough window: measured height is painted height again.
#[test]
fn measured_height_equals_painted_height() {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "hello");
    app.transcript
        .push(BlockKind::Assistant, "hi there, here is the answer");
    let width = 80;
    let total = app.transcript.total_lines(width);
    let painted = app
        .transcript
        .wide_view(width, total, &app.wide_table)
        .len();
    assert_eq!(
        total, painted,
        "measured height ({total}) must equal painted height ({painted})"
    );
}
