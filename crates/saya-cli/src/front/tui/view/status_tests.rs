//! The status line's contract, read as the words a terminal would show.
//!
//! The rows of `02` §4: at 100, 80 and 60 columns the same facts, giving way in
//! the row's drop order — the workspace first, then the meter, the model, the
//! deny list, the database's extra count, the job count, the hint, and the
//! sharing caveat's detail — while the chip and both caveats stay.

use saya_agent::PermissionMode;

use super::status::{StatusFacts, status_line};
use super::status_fixture::{assert_row, bare, cells, full, row, words};
use crate::front::tui::theme::{Charset, Scheme};
use ratatui::style::Modifier;

/// The full row at 100 columns: the workspace, the meter and the model give
/// way; the caveats, the database count and the jobs stay.
#[test]
fn a_hundred_columns_keep_the_caveats_the_count_and_the_jobs() {
    let line = status_line(&full(), 100);

    assert_row(
        &line,
        100,
        "ask │ demo +1 │ ⚠ host unsandboxed │ ⚠ data sharing on │ jobs 1",
        "● working 6s · Esc stop",
    );
    let text = row(&line);
    for dropped in ["~/shop", "ctx 23%", "deepseek-v4.1-flash", "deny:"] {
        assert!(
            !text.contains(dropped),
            "{dropped} should be gone: {text:?}"
        );
    }
}

/// At 80 the database shortens to its count and the jobs go: the caveats, the
/// chip and the state word are still there.
#[test]
fn eighty_columns_drop_the_jobs_before_a_caveat() {
    let line = status_line(&full(), 80);

    assert_row(
        &line,
        80,
        "ask │ +1 │ ⚠ host unsandboxed │ ⚠ data sharing on",
        "● working 6s · Esc stop",
    );
    assert!(
        !row(&line).contains("jobs"),
        "the job count gives way before the caveats: {:?}",
        row(&line)
    );
}

/// At 60 the hint goes and the sharing caveat shortens, and both caveats are
/// still on the row — the one thing the drop order may never cost.
#[test]
fn sixty_columns_cannot_lose_a_caveat() {
    let line = status_line(&full(), 60);

    assert_row(
        &line,
        60,
        "ask │ +1 │ ⚠ host unsandboxed │ ⚠ sharing",
        "● working 6s",
    );
    let text = row(&line);
    assert!(
        text.contains("host unsandboxed"),
        "the unsandboxed caveat is never dropped: {text:?}"
    );
    assert!(
        text.contains("⚠ sharing"),
        "the sharing caveat shortens rather than leaves: {text:?}"
    );
    assert!(
        !text.contains("Esc stop"),
        "the hint goes before the state word: {text:?}"
    );
}

/// The bypass chip is the word, not the hue: with colour off it still reads
/// `! BYPASS`, in reverse video, and no width takes it away.
#[test]
fn the_bypass_chip_reads_its_word_with_colour_off() {
    let facts = StatusFacts {
        mode: PermissionMode::Bypass,
        scheme: Scheme::NoColour,
        ..full()
    };
    let line = status_line(&facts, 60);

    assert_row(
        &line,
        60,
        "! BYPASS │ +1 │ ⚠ host unsandboxed │ ⚠ sharing",
        "● working 6s",
    );
    let chip = line.spans.first().expect("the chip leads the row");
    assert_eq!(chip.content.as_ref(), "! BYPASS");
    assert_eq!(
        chip.style.fg, None,
        "colour off paints the chip no hue at all"
    );
    assert!(
        chip.style.add_modifier.contains(Modifier::REVERSED),
        "reverse video is what is left of the banner: {:?}",
        chip.style
    );
}

/// The two absences are words, not blanks: `no database` and `no workspace`.
#[test]
fn a_missing_database_and_workspace_are_named() {
    let line = status_line(&bare(), 100);

    assert_row(
        &line,
        100,
        "ask │ no database │ qwen3-coder │ no workspace",
        "? help",
    );
    assert!(
        words(&line).contains("no database"),
        "an unattached session says so: {:?}",
        words(&line)
    );
}

/// A toast is the surface's own words: for the four seconds it lives it is the
/// right side, and the state's word and hint are not.
#[test]
fn a_toast_replaces_the_right_side() {
    let facts = StatusFacts {
        toast: Some("copied answer (1.2 kB)".to_owned()),
        ..full()
    };
    let line = status_line(&facts, 100);

    assert_row(
        &line,
        100,
        "ask │ demo +1 │ ⚠ host unsandboxed │ ⚠ data sharing on │ jobs 1",
        "copied answer (1.2 kB)",
    );
    let text = row(&line);
    for gone in ["working", "Esc stop"] {
        assert!(
            !text.contains(gone),
            "{gone} waits for the toast to end: {text:?}"
        );
    }
}

/// `--ascii` reaches the caveat mark too: the row keeps saying the same thing
/// in a seven-bit glyph set.
#[test]
fn the_ascii_row_says_the_same_things() {
    let facts = StatusFacts {
        charset: Charset::Ascii,
        ..full()
    };
    let line = status_line(&facts, 100);

    assert_row(
        &line,
        100,
        "ask │ demo +1 │ ! host unsandboxed │ ! data sharing on │ jobs 1",
        "* working 6s · Esc stop",
    );
    assert_eq!(
        cells(&line),
        100,
        "the ASCII row is measured in the same cells: {:?}",
        row(&line)
    );
}
