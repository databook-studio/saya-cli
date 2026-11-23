//! The two review sizes, one case each: the same consent call, the same panel,
//! read in both seats it can hold the keyboard in (UI3).
//!
//! Each case prints the screen it read before it asserts anything, so the
//! capture in `REPORT.md` is the terminal's own text and not a re-drawing of it
//! by the test. Every case reads the panel twice: with the composer holding the
//! keyboard (`Tab to answer`), and with the panel holding it (`↑↓ choose`).

use super::DEADLINE;
use super::PAINT;
use super::harness::{NARROW, Panel, Size, WIDE};

/// The call this surface is asked to answer: a coding job's start consent.
const CALL: &str = "/start --id pty-job 'approval panel review' --write a.txt";

/// The row that says the panel holds the keyboard.
const PANEL_FOCUSED: &str = "↑↓ choose";
/// The row that says the composer does.
const COMPOSER_FOCUSED: &str = "Tab to answer";

#[test]
fn the_panel_is_a_bordered_full_width_block_at_one_hundred_by_thirty() {
    review("wide", WIDE);
}

#[test]
fn the_panel_is_readable_at_sixty_by_twenty_four() {
    review("narrow", NARROW);
}

/// Opens the surface, asks for one job, and reads the panel it puts up — first
/// with the composer holding the keyboard, then with the panel holding it.
fn review(label: &str, size: Size) {
    let mut panel = Panel::open(label, size);
    panel.wait_for("? help", PAINT);
    panel.send_line(CALL);
    panel.wait_for("Approve job pty-job", DEADLINE);
    // The proposal paints in more than one frame, and a frame in more than one
    // chunk: the capture is the settled screen, not a frame caught mid-write.
    panel.wait_for("Approve this job?", PAINT);
    panel.settle();

    // A call can open while the operator is typing, so the seat it takes
    // depends on the line and the clock. Both are read here, and the keyboard
    // is moved by the key the panel names rather than assumed.
    let unfocused = composer_seat(&mut panel);
    println!(
        "---- {label} {}x{} · composer seat ----\n{unfocused}",
        size.cols, size.rows
    );
    let lines: Vec<&str> = unfocused.lines().collect();
    let title = position(&lines, "Approve this job?");
    let top = lines[title];
    assert!(
        top.trim_start().starts_with('╭') && top.trim_end().ends_with('╮'),
        "{label}: the panel opens with a full-width rule: {top:?}"
    );
    assert!(
        top.chars().count() >= usize::from(size.cols) - 3,
        "{label}: the panel spans the screen's width: {} of {}",
        top.chars().count(),
        size.cols
    );
    // The call itself, exactly as the job stated it.
    let body = position(&lines, "Approve job pty-job with this exact scope:");
    assert!(body > title, "{label}: the call sits under the title");
    assert!(
        position(&lines, "write: a.txt") > body,
        "{label}: the write scope is stated under the call"
    );
    // The proposal's own bytes are not in the body an operator reads.
    assert!(
        !unfocused.contains("{\""),
        "{label}: the raw proposal is behind `v`, not in the body: {unfocused}"
    );
    // Where it runs.
    let folder = position(&lines, "in ");
    assert!(
        lines[folder].contains("repo"),
        "{label}: the folder the call runs in is named: {:?}",
        lines[folder]
    );
    // The answers, one per line, in the order the panel offers them.
    let allow = position(&lines, "[y] Allow once");
    let deny = position(&lines, "[n] Deny");
    let escape = position(&lines, "[Esc] Deny");
    assert!(
        allow > folder && deny > allow && escape > deny,
        "{label}: the choices are on their own lines, in order: {unfocused}"
    );
    assert!(
        !unfocused.contains("[s]"),
        "{label}: a call with no session grant offers no session answer: {unfocused}"
    );
    // The composer holds the keyboard here: it takes a draft, and names the key
    // that answers the call above it.
    assert!(
        unfocused.contains(COMPOSER_FOCUSED),
        "{label}: the unfocused panel names the key that answers it: {unfocused}"
    );
    assert!(
        unfocused.contains("Tab answer the approval"),
        "{label}: and the composer it left enabled names it too: {unfocused}"
    );
    assert!(
        !unfocused.contains("Message (disabled"),
        "{label}: a composer with the keyboard is not disabled: {unfocused}"
    );

    // `Tab`: the panel takes the keyboard, and answers where it stands.
    panel.send(b"\t");
    panel.wait_for(PANEL_FOCUSED, PAINT);
    panel.settle();
    let focused = panel.screen();
    println!(
        "---- {label} {}x{} · panel seat ----\n{focused}",
        size.cols, size.rows
    );
    let lines: Vec<&str> = focused.lines().collect();
    assert!(
        focused.contains(PANEL_FOCUSED),
        "{label}: the focused panel names the arrows it reads: {focused}"
    );
    assert!(
        !focused.contains(COMPOSER_FOCUSED),
        "{label}: and no longer says the keyboard is elsewhere: {focused}"
    );
    let escape = position(&lines, "[Esc] Deny");
    let composer = position(&lines, "Message (disabled");
    assert!(
        composer > escape,
        "{label}: the composer says it is disabled under the panel: {focused}"
    );

    // `v`: the exact proposal the yes covers, the one thing the body withheld.
    panel.send(b"v");
    panel.wait_for("{\"", PAINT);
    panel.settle();
    let opened = panel.screen();
    println!(
        "---- {label} {}x{} · v ----\n{opened}",
        size.cols, size.rows
    );
    assert!(
        opened.contains("{\""),
        "{label}: `v` opens the proposal's own bytes: {opened}"
    );
    let _ = DEADLINE;
}

/// The panel with the composer holding the keyboard: `Tab` moves it there
/// whenever the call took it, and the screen says which seat it is in.
fn composer_seat(panel: &mut Panel) -> String {
    if !panel.screen().contains(COMPOSER_FOCUSED) {
        panel.send(b"\t");
        panel.wait_for(COMPOSER_FOCUSED, PAINT);
        panel.settle();
    }
    panel.screen()
}

/// The row `needle` sits on.
fn position(lines: &[&str], needle: &str) -> usize {
    lines
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} is not on screen:\n{}", lines.join("\n")))
}
