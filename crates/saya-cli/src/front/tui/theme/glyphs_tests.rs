//! `02` §11's glyph table, both columns, asserted.
//!
//! The table is transcribed here from the spec rather than from the
//! implementation: a glyph whose ASCII form stops matching `02` §11 — or that
//! loses its fallback — fails, however the code got there.

use crate::front::tui::theme::{Charset, Glyph, charset, glyph};

/// `02` §11, row by row: every glyph the new TUI draws, and the ASCII form that
/// replaces it under `--ascii` or a non-UTF-8 locale.
const TABLE: [(Glyph, &str, &str); 18] = [
    (Glyph::Prompt, "❯", ">"),
    (Glyph::Check, "✓", "[ok]"),
    (Glyph::Cross, "✗", "[x]"),
    (Glyph::Denied, "⊘", "[no]"),
    (Glyph::Ask, "?", "[?]"),
    (Glyph::Bullet, "●", "*"),
    (Glyph::Paused, "⏸", "[||]"),
    (Glyph::Stop, "■", "[#]"),
    (Glyph::Edit, "✎", "[ed]"),
    (Glyph::Hint, "◆", "[h]"),
    (Glyph::TopLeft, "╭", "+"),
    (Glyph::TopRight, "╮", "+"),
    (Glyph::BottomLeft, "╰", "+"),
    (Glyph::BottomRight, "╯", "+"),
    (Glyph::Rule, "─", "-"),
    (Glyph::Edge, "│", "|"),
    (Glyph::Selected, "▸", ">"),
    (Glyph::Collapsed, "▾", "v"),
];

#[test]
fn every_glyph_of_02_11_has_the_ascii_form_the_table_gives() {
    assert_eq!(
        TABLE.len(),
        Glyph::ALL.len(),
        "the glyph set and 02 §11's table disagree on how many glyphs there are"
    );
    for (shape, utf8, ascii) in TABLE {
        assert!(
            Glyph::ALL.contains(&shape),
            "{} is in the table but not in the glyph set",
            shape.name()
        );
        assert_eq!(
            glyph(shape, Charset::Utf8),
            utf8,
            "{}'s UTF-8 form",
            shape.name()
        );
        assert_eq!(
            glyph(shape, Charset::Ascii),
            ascii,
            "{}'s ASCII form",
            shape.name()
        );
    }
}

#[test]
fn every_ascii_form_is_seven_bit_and_non_empty() {
    for shape in Glyph::ALL {
        let ascii = glyph(shape, Charset::Ascii);
        assert!(!ascii.is_empty(), "{} has no ASCII form", shape.name());
        assert!(
            ascii.is_ascii(),
            "{}'s ASCII form {ascii:?} is not seven-bit, so it cannot survive --ascii",
            shape.name()
        );
    }
}

#[test]
fn the_ascii_fallback_replaces_every_glyph() {
    for shape in Glyph::ALL {
        assert_ne!(
            glyph(shape, Charset::Ascii),
            glyph(shape, Charset::Utf8),
            "{} draws the same in both charsets, so --ascii swaps nothing",
            shape.name()
        );
    }
}

#[test]
fn the_utf8_set_is_what_the_screens_draw() {
    for shape in Glyph::ALL {
        let utf8 = glyph(shape, Charset::Utf8);
        assert!(!utf8.is_empty(), "{} has no UTF-8 form", shape.name());
        assert!(
            !utf8.is_ascii() || shape == Glyph::Ask,
            "{}'s UTF-8 form {utf8:?} is plain ASCII, so it is not the glyph 02 §11 draws",
            shape.name()
        );
    }
}

#[test]
fn ascii_flag_or_a_non_utf8_locale_selects_the_fallback() {
    assert_eq!(charset(true, true), Charset::Ascii, "--ascii always swaps");
    assert_eq!(
        charset(true, false),
        Charset::Ascii,
        "--ascii wins even in a UTF-8 locale"
    );
    assert_eq!(
        charset(false, false),
        Charset::Ascii,
        "a non-UTF-8 locale swaps on its own"
    );
    assert_eq!(
        charset(false, true),
        Charset::Utf8,
        "a UTF-8 locale with no flag keeps the glyphs"
    );
}
