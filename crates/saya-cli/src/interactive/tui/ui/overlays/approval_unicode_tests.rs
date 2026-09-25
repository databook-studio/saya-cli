//! Wide and grapheme-heavy bodies are measured as painted (re-audit R01).
//! The panel decided its layout with a char-based row count and painted with
//! ratatui's cell-aware wrap: with wide characters the two disagreed, and the
//! panel asked for consent while hiding the thing being consented to —
//! controls off-screen, final facts below an unreachable end-stop. Real
//! frames through the real `ui::draw`, asserted on the buffer.

use crate::interactive::tui::types::{App, PendingApproval};
use crate::interactive::tui::ui_snapshot_tests::{empty_app, fixed_status, render_buffer};

/// The re-audit's body shape: `fact line NN ` followed by `日本` ×20 — 13
/// chars of prefix plus 40 chars that occupy 80 cells, so the char count (53)
/// and the painted width (93) disagree at the panel's inner width.
fn wide_detail(lines: usize) -> String {
    (1..=lines)
        .map(|i| format!("fact line {i:02} {}", "日本".repeat(20)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The same body in equal-length ASCII — the control the re-audit used to
/// isolate the cause: cells == chars here, so the char arithmetic was never
/// wrong for it.
fn ascii_detail(lines: usize) -> String {
    (1..=lines)
        .map(|i| format!("fact line {i:02} {}", "x".repeat(80)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// An idle app with a pending approval carrying `detail` and no grant, at the
/// given scroll offset.
fn approval_app(detail: String, scroll: usize) -> App {
    let mut app = empty_app();
    let (respond, _answer) = tokio::sync::oneshot::channel();
    app.request.pending_approval = Some(PendingApproval {
        tool: "http_fetch".into(),
        detail: Some(detail),
        grant: None,
        scroll,
        respond,
    });
    app
}

/// The security-relevant repro: 20 wide logical lines whose char count (20)
/// equals the answers' budget while their painted rows (60) do not — the
/// counter picked the flat-fit branch, so the later facts **and the
/// `[a]`/`[d]` controls** were clipped and scrolling was ignored. The panel
/// is asking for consent with the controls off-screen.
///
/// The frame is taller than the classic 80×24 floor on purpose: at 80×24 the
/// layout squeezes the panel below its 24-row request (the transcript's
/// `Min(1)` loses), the budget shrinks, and this same body already lands in
/// the overflow branch — where the tests below pin the defect. The false fit
/// the re-audit executed needs the panel at its requested height, so this
/// frame grants it.
#[test]
fn a_wide_body_that_falsely_fits_still_shows_its_controls() {
    let app = approval_app(wide_detail(20), 0);
    let buffer = render_buffer(&app, &fixed_status(), 80, 30);
    assert!(
        buffer.contains("[a] allow once"),
        "the [a] control is painted for a wide body that falsely fits:\n{buffer}"
    );
    assert!(
        buffer.contains("[d] deny"),
        "the [d] control is painted for a wide body that falsely fits:\n{buffer}"
    );
}

/// 30 wide lines: the overflow branch keeps the controls, but the char-count
/// end-stop still cannot reach the final fact. At maximum scroll the final
/// fact's text must be painted.
#[test]
fn a_wide_body_reaches_its_final_fact_at_maximum_scroll() {
    let app = approval_app(wide_detail(30), usize::MAX);
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("fact line 30"),
        "the final fact is painted at maximum scroll:\n{buffer}"
    );
}

/// The control, passing before and after the fix: the same body in
/// equal-length ASCII reaches its final fact. Cells equal chars here, so a
/// failure means the change broke ASCII measurement — the snapshot is right.
#[test]
fn an_ascii_body_of_equal_length_still_reaches_its_final_fact() {
    let app = approval_app(ascii_detail(30), usize::MAX);
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("fact line 30"),
        "the equal-length ASCII control still reaches its final fact:\n{buffer}"
    );
}

/// The other two grapheme classes the transcript's cell-aware wrap covers:
/// combining marks (several chars, one cell) and ZWJ emoji sequences
/// (several chars, two cells) must reach the final fact at maximum scroll —
/// a char counter overshoots the painted rows for both.
#[test]
fn combining_marks_and_emoji_reach_the_final_fact() {
    let combining = (1..=30)
        .map(|i| format!("fact line {i:02} {}", "e\u{0301}".repeat(40)))
        .collect::<Vec<_>>()
        .join("\n");
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let emoji = (1..=30)
        .map(|i| format!("fact line {i:02} {}", family.repeat(20)))
        .collect::<Vec<_>>()
        .join("\n");
    for (kind, detail) in [("combining marks", combining), ("ZWJ emoji", emoji)] {
        let app = approval_app(detail, usize::MAX);
        let buffer = render_buffer(&app, &fixed_status(), 80, 24);
        assert!(
            buffer.contains("fact line 30"),
            "a {kind} body reaches its final fact at maximum scroll:\n{buffer}"
        );
    }
}

/// A long unbroken prose line wraps into several rows and the count must be
/// what is painted: at offset 0 the marker names the rows the paint really
/// has, and at the end-stop the final row is on screen with nothing withheld.
/// The non-Unicode half of the same measurement/paint bug.
///
/// Geometry, derived: 2000 `w`s wrap into 26 rows at the panel's inner 78
/// columns (25 rows of 78 plus a 50-char tail); at 80×24 the panel gets 18
/// rows (inner 16), so the answers' budget is 14 — 13 visible body rows plus
/// the marker row — and the marker reads 26 − 13 = 13 withheld.
#[test]
fn prose_wrapping_is_measured_as_painted() {
    let detail = "w".repeat(2000);
    let buffer = render_buffer(&approval_app(detail.clone(), 0), &fixed_status(), 80, 24);
    assert!(
        buffer.contains("13 more lines hidden"),
        "the marker counts the rows the paint really has:\n{buffer}"
    );
    let buffer = render_buffer(&approval_app(detail, usize::MAX), &fixed_status(), 80, 24);
    assert!(
        buffer.contains(&"w".repeat(40)),
        "the tail of the prose paints at maximum scroll:\n{buffer}"
    );
    assert!(
        !buffer.contains("more lines hidden"),
        "the end-stop lands on the last painted row, nothing withheld:\n{buffer}"
    );
}

/// The offered choices are visible at every offset of the 30-line wide body.
/// The sweep walks 0..=100, covering both the char-count end-stop and the
/// corrected one, clamped past either end.
#[test]
fn the_controls_are_visible_at_every_offset() {
    for scroll in 0..=100usize {
        let app = approval_app(wide_detail(30), scroll);
        let buffer = render_buffer(&app, &fixed_status(), 80, 24);
        assert!(
            buffer.contains("[a] allow once"),
            "the [a] control is visible at offset {scroll}:\n{buffer}"
        );
        assert!(
            buffer.contains("[d] deny"),
            "the [d] control is visible at offset {scroll}:\n{buffer}"
        );
    }
}

/// A short terminal — well below the classic 80×24 floor, so the layout
/// squeezes the panel itself — still shows the controls: the answers'
/// reservation wins whatever room is left.
#[test]
fn a_short_terminal_still_shows_the_controls() {
    let app = approval_app(wide_detail(30), 0);
    let buffer = render_buffer(&app, &fixed_status(), 80, 12);
    assert!(
        buffer.contains("[a] allow once"),
        "the [a] control survives a short terminal:\n{buffer}"
    );
    assert!(
        buffer.contains("[d] deny"),
        "the [d] control survives a short terminal:\n{buffer}"
    );
}
