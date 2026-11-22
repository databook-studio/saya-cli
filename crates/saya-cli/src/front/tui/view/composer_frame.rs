//! The composer's frame: the box `02` §7 draws around the state's words.
//!
//! The copy and the state table live in [`super::composer`]; this module only
//! lays rows out — a top border with the title, one row per wrapped body line,
//! and a bottom border carrying the hint. The wrap is today's
//! [`InputBuffer`], so the pane and the cursor can never disagree about where a
//! line breaks, and every glyph comes from the theme, so `--ascii` needs no
//! second path.

use crate::front::tui::model::Model;
use crate::front::tui::theme::{Charset, Glyph, Token, glyph};
use crate::interactive::tui::InputBuffer;

use super::composer::{
    Row, Run, SEARCH_HINT, State, hint, placeholder, search_prompt, state, title,
};

/// The narrowest box the composer will draw: below this the label and the
/// hint are all that can be fitted.
pub(crate) const MIN_WIDTH: usize = 8;

/// The composer box at `width` columns, top border first.
pub(crate) fn rows(model: &Model, width: usize, charset: Charset) -> Vec<Row> {
    let width = width.max(MIN_WIDTH);
    let state = state(model);
    let mut out = vec![border(
        Glyph::TopLeft,
        Glyph::TopRight,
        title(state),
        Token::Accent,
        width,
        charset,
    )];
    let (text, token) = match search_prompt(model) {
        // A search in progress is what the line is: the prompt and the entry it
        // found, in the token a draft is typed in.
        Some(prompt) => (prompt, Token::Fg),
        None => draft(model, state),
    };
    for line in wrapped(&text, width.saturating_sub(4)) {
        out.push(Row {
            runs: vec![Run {
                text: body(&line, width, charset),
                token,
            }],
        });
    }
    out.push(border(
        Glyph::BottomLeft,
        Glyph::BottomRight,
        // The state's hint names keys a search does not read, so a search says
        // its own row instead.
        if model.search().is_some() {
            SEARCH_HINT
        } else {
            hint(state, width)
        },
        Token::Dim,
        width,
        charset,
    ));
    out
}

/// The text the body shows and the token it is painted with: the held line
/// while one is queued, otherwise the draft or, when it is empty, the
/// placeholder.
fn draft(model: &Model, state: State) -> (String, Token) {
    if state == State::OneQueued {
        return (model.queued().unwrap_or_default().to_owned(), Token::Dim);
    }
    if model.composer().is_empty() {
        (placeholder(model), Token::Dim)
    } else {
        (model.composer().to_owned(), Token::Fg)
    }
}

/// One logical body text wrapped to `room` columns by today's input buffer.
fn wrapped(text: &str, room: usize) -> Vec<String> {
    let mut buffer = InputBuffer::new();
    buffer.set_text(text);
    buffer.wrapped_lines(room.max(1))
}

/// A body row: the edges, and the text padded to the box's width.
fn body(text: &str, width: usize, charset: Charset) -> String {
    let edge = glyph(Glyph::Edge, charset);
    let room = width.saturating_sub(4);
    format!("{edge} {} {edge}", pad(text, room))
}

/// A border row: a corner, a label, the rule and the other corner.
fn border(
    left: Glyph,
    right: Glyph,
    label: &str,
    token: Token,
    width: usize,
    charset: Charset,
) -> Row {
    let label = fit(label, width.saturating_sub(4));
    let fill = width.saturating_sub(label.chars().count() + 4);
    let rule = glyph(Glyph::Rule, charset);
    Row {
        runs: vec![
            Run {
                text: format!("{} ", glyph(left, charset)),
                token: Token::Dim,
            },
            Run { text: label, token },
            Run {
                text: format!(" {}{}", rule.repeat(fill), glyph(right, charset)),
                token: Token::Dim,
            },
        ],
    }
}

/// Fits a label or a line to `room` columns, marking the cut with an ellipsis
/// rather than letting it spill over the border.
fn fit(text: &str, room: usize) -> String {
    if text.chars().count() <= room {
        return text.to_owned();
    }
    if room == 0 {
        return String::new();
    }
    let kept: String = text.chars().take(room - 1).collect();
    format!("{kept}…")
}

/// Pads `text` to `room` columns: the box's body keeps its right edge in line
/// with every other row.
fn pad(text: &str, room: usize) -> String {
    let fitted = fit(text, room);
    let used = fitted.chars().count();
    format!("{fitted}{}", " ".repeat(room.saturating_sub(used)))
}
