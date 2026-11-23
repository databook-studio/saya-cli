//! The two review sizes, one case each: the same consent call, the same panel.
//!
//! Each case prints the screen it read before it asserts anything, so the
//! capture in `REPORT.md` is the terminal's own text and not a re-drawing of
//! it by the test.

use super::DEADLINE;
use super::PAINT;
use super::harness::{NARROW, Panel, Size, WIDE};

/// The call this surface is asked to answer: a coding job's start consent.
const CALL: &str = "/start --id pty-job 'approval panel review' --write a.txt";

#[test]
fn the_panel_is_a_bordered_full_width_block_at_one_hundred_by_thirty() {
    review("wide", WIDE);
}

#[test]
fn the_panel_is_readable_at_sixty_by_twenty_four() {
    review("narrow", NARROW);
}

/// Opens the surface, asks for one job, and reads the panel it puts up.
fn review(label: &str, size: Size) {
    let mut panel = Panel::open(label, size);
    panel.wait_for("? help", PAINT);
    panel.send_line(CALL);
    panel.wait_for("Approve job pty-job", DEADLINE);
    // The proposal paints in more than one frame; the box is the last row to
    // arrive, so the panel is complete when its title is on screen.
    panel.wait_for("Approve this job?", PAINT);

    let screen = panel.screen();
    println!("---- {label} {}x{} ----\n{screen}", size.cols, size.rows);

    let lines: Vec<&str> = screen.lines().collect();
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
        "{label}: the choices are on their own lines, in order: {screen}"
    );
    assert!(
        !screen.contains("[s]"),
        "{label}: a call with no session grant offers no session answer: {screen}"
    );
    // The composer gives way while the call waits.
    let composer = position(&lines, "Message (disabled");
    assert!(
        composer > escape,
        "{label}: the composer says it is disabled under the panel: {screen}"
    );
    let _ = DEADLINE;
}

/// The row `needle` sits on.
fn position(lines: &[&str], needle: &str) -> usize {
    lines
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} is not on screen:\n{}", lines.join("\n")))
}
