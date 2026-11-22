//! The seven-bit fallback for text no glyph of `02` §11 names.
//!
//! The theme's [`Glyph`](super::glyphs::Glyph) table covers the shapes the
//! screens draw on purpose; a table's border, a copy `·`, a `…` from a
//! truncation and the `⚠` a pane writes inline are copy, not glyphs, and a
//! terminal that cannot draw the UTF-8 set still has to read them. The frame
//! passes every row through [`line`] when the charset is
//! [`Charset::Ascii`](super::glyphs::Charset), so one table owns the answer and
//! no pane keeps a second copy of it.

use ratatui::text::{Line, Span};

/// One row in the seven-bit set, its spans' styles untouched.
pub(crate) fn line(line: Line<'static>) -> Line<'static> {
    Line::from(
        line.spans
            .into_iter()
            .map(|span| {
                let style = span.style;
                Span::styled(text(span.content.as_ref()), style)
            })
            .collect::<Vec<_>>(),
    )
}

/// `text` with every non-ASCII character replaced by its ASCII form.
pub(crate) fn text(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for character in source.chars() {
        if character.is_ascii() {
            out.push(character);
        } else {
            out.push_str(ascii_form(character));
        }
    }
    out
}

/// The ASCII form of one character. The eighteen glyphs of `02` §11 use the
/// table's own fallback; the rest are the punctuation and arrows the copy
/// writes, so the mapping is stated once here rather than in each pane.
fn ascii_form(character: char) -> &'static str {
    match character {
        // `02` §11's glyph table, for a pane that wrote the UTF-8 form inline.
        '❯' => ">",
        '✓' => "[ok]",
        '✗' => "[x]",
        '⊘' => "[no]",
        '●' => "*",
        '⏸' => "[||]",
        '■' => "[#]",
        '✎' => "[ed]",
        '◆' => "[h]",
        '▸' => ">",
        '▾' => "v",
        // Box drawing: the table's frames and today's table renderer.
        '│' | '┃' => "|",
        '─' | '━' => "-",
        '╭' | '╮' | '╰' | '╯' | '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼' | '╞' | '╡' => {
            "+"
        }
        // Copy punctuation.
        '·' => "*",
        '—' | '–' | '−' => "-",
        '…' => "...",
        '↑' => "^",
        '↓' => "v",
        '↵' => "<",
        '›' => ">",
        '◌' => "o",
        '⚠' => "!",
        '’' => "'",
        '‘' => "'",
        '“' | '”' => "\"",
        '×' => "x",
        '\u{a0}' => " ",
        // A character this table never met still cannot reach a terminal that
        // cannot draw it.
        _ => "?",
    }
}
