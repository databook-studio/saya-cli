//! The status line's contract, read as the words a terminal would show.
//!
//! The row says the mode, the short model and the database, and nothing else by
//! default: the unsandboxed warning belongs to the approval card, the toast has
//! a row of its own, and a mode change never takes the model name away.

use saya_agent::PermissionMode;

use super::status::{ActivityState, StatusFacts, status_line};
use super::status_fixture::{assert_row, bare, cells, full, row, words};
use crate::front::tui::theme::{Charset, Scheme};
use ratatui::style::Modifier;

/// The full row at 100 columns: mode, short model, database and the one
/// conditional caveat, then the state.
#[test]
fn a_hundred_columns_read_mode_model_database() {
    let line = status_line(&full(), 100);

    assert_row(
        &line,
        100,
        "ask │ deepseek-v4.1-flash │ demo +1 │ ⚠ data sharing on",
        "● working 6s · Esc stop",
    );
    let text = row(&line);
    for gone in [
        "~/shop",
        "ctx",
        "deny",
        "jobs",
        "host unsandboxed",
        "deepseek/",
    ] {
        assert!(
            !text.contains(gone),
            "{gone:?} has no place on the row: {text:?}"
        );
    }
}

/// At 60 the extra count and the caveat's detail give way; the mode, the model
/// and the database stay.
#[test]
fn sixty_columns_keep_the_model_and_the_database() {
    let line = status_line(&full(), 60);

    assert_row(
        &line,
        60,
        "ask │ deepseek-v4.1-flash │ +1 │ ⚠ sharing",
        "● working 6s",
    );
    let text = words(&line);
    assert!(
        text.contains("deepseek-v4.1-flash"),
        "the model name survives the narrow row: {text:?}"
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
    let line = status_line(&facts, 80);

    assert!(
        row(&line).starts_with("! BYPASS │ deepseek-v4.1-flash │ "),
        "the chip leads the row: {:?}",
        row(&line)
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

/// The missing database is a word, not a blank.
#[test]
fn a_missing_database_is_named() {
    let line = status_line(&bare(), 100);

    assert_row(&line, 100, "ask │ qwen3-coder │ no database", "? help");
}

/// A model with no provider prefix reads as it is, and an empty model paints no
/// segment at all.
#[test]
fn the_model_segment_is_optional() {
    let facts = StatusFacts {
        databases: vec![String::from("demo")],
        model: String::from("qwen3-coder"),
        ..StatusFacts::default()
    };
    assert_eq!(
        row(&status_line(&facts, 40)),
        "ask │ qwen3-coder │ demo          ? help",
        "a plain model name keeps its segment"
    );

    let facts = StatusFacts {
        databases: vec![String::from("demo")],
        ..StatusFacts::default()
    };
    assert!(
        row(&status_line(&facts, 40)).starts_with("ask │ demo"),
        "no model, no segment: {:?}",
        row(&status_line(&facts, 40))
    );
}

/// A been-running turn says its seconds, and an idle surface offers the sheet.
#[test]
fn the_state_word_outlives_its_hint() {
    let facts = StatusFacts {
        activity: ActivityState::Queued { count: 1 },
        mode: PermissionMode::Plan,
        ..StatusFacts::default()
    };
    let wide = status_line(&facts, 60);
    assert!(
        row(&wide).ends_with("queued 1 · Ctrl+G drop"),
        "the hint rides beside the state word while there is room: {:?}",
        row(&wide)
    );

    let facts = StatusFacts {
        model: "deepseek-v4.1-flash".to_owned(),
        ..facts
    };
    let narrow = status_line(&facts, 60);

    let text = row(&narrow);
    assert!(
        text.ends_with("queued 1"),
        "the state word outlives its hint: {text:?}"
    );
    assert!(
        !text.contains("Ctrl+G drop"),
        "the hint is dropped first: {text:?}"
    );
    assert!(cells(&narrow) <= 60, "the row fits: {text:?}");
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
        "ask │ deepseek-v4.1-flash │ demo +1 │ ! data sharing on",
        "* working 6s · Esc stop",
    );
    assert_eq!(
        cells(&line),
        100,
        "the ASCII row is measured in the same cells: {:?}",
        row(&line)
    );
}
