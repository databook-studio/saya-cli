//! Fenced code in an answer, read as the runs a terminal paints: the keyword,
//! the string, the comment, the number, and everything else plain.
//!
//! One line at a time, and never more than a shape: the reader wants the line's
//! shape, and a construct this table does not know paints plain rather than
//! wrong. Under `--ascii` and with colour off the runs are the same words — the
//! pane is still the answer's text, only unpainted.

use ratatui::style::Style;
use ratatui::text::Span;

use super::highlight_langs::{Language, dialect, language};
use crate::front::tui::theme::{Scheme, Token, style};

/// What one run of a code line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Everything the table does not name.
    Plain,
    /// A word the language reserves.
    Keyword,
    /// A quoted string.
    Str,
    /// A comment, to the end of the line.
    Comment,
    /// A numeric literal.
    Number,
}

/// One run of a code line: its words and what they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) text: String,
    pub(crate) kind: Kind,
}

/// The fence state a renderer carries across an answer's lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Fence {
    open: bool,
    language: Option<Language>,
}

impl Fence {
    /// Reads one line as a fence marker: `Some(true)` opens a block, `Some(false)`
    /// closes one, `None` is a line of the answer. The word after the opening
    /// fence names the block's language; an unknown one leaves the block plain.
    pub(crate) fn toggle(&mut self, line: &str) -> Option<bool> {
        let trimmed = line.trim_start();
        let rest = trimmed.strip_prefix("```")?;
        self.open = !self.open;
        self.language = if self.open {
            let name = rest.split_whitespace().next().unwrap_or_default();
            language(name)
        } else {
            None
        };
        Some(self.open)
    }

    /// Whether the line being read is inside a block.
    pub(crate) fn open(&self) -> bool {
        self.open
    }

    /// The open block's language, when the fence named one this table knows.
    pub(crate) fn language(&self) -> Option<Language> {
        self.language
    }

    /// The marker row a fence line paints, in place of its backticks.
    pub(crate) fn marker(&self, scheme: Scheme) -> Span<'static> {
        Span::styled("···".to_string(), style(Token::Dim, scheme))
    }
}

/// The runs of one code line, in the order the line reads them. A line of an
/// unknown or unnamed language is one plain run.
pub(crate) fn runs(line: &str, language: Option<Language>) -> Vec<Run> {
    let Some(language) = language else {
        return vec![Run {
            text: line.to_string(),
            kind: Kind::Plain,
        }];
    };
    let dialect = dialect(language);
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if let Some(marker) = dialect.comment
            && starts_at(&chars, index, marker)
        {
            let rest: String = chars[index..].iter().collect();
            push(&mut out, rest, Kind::Comment);
            break;
        }
        let character = chars[index];
        if dialect.quotes.contains(&character) {
            let end = closing(&chars, index, character);
            let text: String = chars[index..end].iter().collect();
            push(&mut out, text, Kind::Str);
            index = end;
        } else if character.is_ascii_digit() {
            let end = number_end(&chars, index);
            let text: String = chars[index..end].iter().collect();
            push(&mut out, text, Kind::Number);
            index = end;
        } else if character.is_alphanumeric() || character == '_' {
            let end = word_end(&chars, index);
            let word: String = chars[index..end].iter().collect();
            let kind = if is_keyword(&word, dialect.keywords, dialect.caseless) {
                Kind::Keyword
            } else {
                Kind::Plain
            };
            push(&mut out, word, kind);
            index = end;
        } else {
            push(&mut out, character.to_string(), Kind::Plain);
            index += 1;
        }
    }
    out
}

/// The spans one code line paints in `scheme`.
pub(crate) fn spans(line: &str, language: Option<Language>, scheme: Scheme) -> Vec<Span<'static>> {
    runs(line, language)
        .into_iter()
        .map(|run| Span::styled(run.text, paint(run.kind, scheme)))
        .collect()
}

/// What one kind paints: a hue while colour is on, and the plain code style
/// when it is not — colour going away changes no word of the line.
fn paint(kind: Kind, scheme: Scheme) -> Style {
    if scheme == Scheme::NoColour {
        return style(Token::Code, scheme);
    }
    match kind {
        Kind::Plain => style(Token::Code, scheme),
        Kind::Keyword => style(Token::Accent, scheme),
        Kind::Str => style(Token::Ok, scheme),
        Kind::Comment => style(Token::Dim, scheme),
        Kind::Number => style(Token::Info, scheme),
    }
}

/// Whether `word` is one of `keywords`, exactly as the language matches them.
fn is_keyword(word: &str, keywords: &[&str], caseless: bool) -> bool {
    keywords.iter().any(|keyword| {
        if caseless {
            keyword.eq_ignore_ascii_case(word)
        } else {
            *keyword == word
        }
    })
}

/// Whether `marker` starts at `index`.
fn starts_at(chars: &[char], index: usize, marker: &str) -> bool {
    marker
        .chars()
        .enumerate()
        .all(|(offset, character)| chars.get(index + offset) == Some(&character))
}

/// The index just past a string's closing quote, or the line's end when the
/// string is unterminated — a wrapped row can carry only half of one.
fn closing(chars: &[char], start: usize, quote: char) -> usize {
    let mut index = start + 1;
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 2,
            character if character == quote => return index + 1,
            _ => index += 1,
        }
    }
    chars.len()
}

/// The index just past a numeric literal.
fn number_end(chars: &[char], start: usize) -> usize {
    let mut index = start;
    while index < chars.len() {
        let character = chars[index];
        if character.is_ascii_alphanumeric() || matches!(character, '_' | '.') {
            index += 1;
        } else {
            break;
        }
    }
    index
}

/// The index just past a word.
fn word_end(chars: &[char], start: usize) -> usize {
    let mut index = start;
    while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
        index += 1;
    }
    index
}

/// Adds one run, folding it into the last when both are the same kind.
fn push(out: &mut Vec<Run>, text: String, kind: Kind) {
    if let Some(last) = out.last_mut()
        && last.kind == kind
    {
        last.text.push_str(&text);
        return;
    }
    out.push(Run { text, kind });
}

#[cfg(test)]
#[path = "highlight_tests.rs"]
mod tests;
