//! Fenced code, read as the words a terminal paints: three languages, the
//! plain fallback, and colour off.
//!
//! The snapshots pin the *shape* of a line — which run is the keyword, the
//! string, the comment — because that is what the reader sees; the assertions
//! beside them pin the paint, so a run cannot drift to another token silently.

use ratatui::text::Span;

use super::{Fence, Kind, runs, spans};
use crate::front::tui::theme::{Scheme, Token, style};

/// One span as `kind text`, read off the paint rather than the tokeniser, so a
/// snapshot proves what a terminal shows.
fn outline(line: &str, language: Option<super::Language>, scheme: Scheme) -> Vec<String> {
    spans(line, language, scheme)
        .iter()
        .map(|span| {
            let kind = match span.style {
                _ if span.style == style(Token::Accent, scheme) => "keyword",
                _ if span.style == style(Token::Ok, scheme) => "string",
                _ if span.style == style(Token::Dim, scheme) => "comment",
                _ if span.style == style(Token::Info, scheme) => "number",
                _ => "plain",
            };
            format!("{kind:<8} {}", span.content.as_ref())
        })
        .collect()
}

/// The three languages a fenced answer carries most often.
#[test]
fn three_languages_read_as_their_own_shape() {
    insta::assert_debug_snapshot!(
        "rust_line",
        outline(
            "fn main() { let name = \"saya\"; // the entry",
            Some(super::Language::Rust),
            Scheme::Dark,
        )
    );
    insta::assert_debug_snapshot!(
        "python_line",
        outline(
            "def total(items):  # sum them",
            Some(super::Language::Python),
            Scheme::Dark,
        )
    );
    insta::assert_debug_snapshot!(
        "sql_line",
        outline(
            "SELECT id, name FROM users WHERE id = 42; -- one row",
            Some(super::Language::Sql),
            Scheme::Dark,
        )
    );
}

/// A fence naming a language this table does not know — or naming none — is one
/// plain run, exactly as it painted before there was a highlighter.
#[test]
fn an_unknown_language_keeps_the_line_plain() {
    let line = ";; a form (let ((x 1)) x)";
    insta::assert_debug_snapshot!("fallback_line", outline(line, None, Scheme::Dark));
    assert_eq!(
        outline(line, None, Scheme::Dark),
        vec![format!("plain    {line}")],
        "the whole line is one run"
    );
    assert_eq!(
        super::language("cobol"),
        None,
        "an unlisted fence is unknown"
    );
    assert_eq!(super::language(""), None, "a bare fence names nothing");
}

/// Colour off is the same words in the same order: no run carries a hue, and
/// joining them gives the line back character for character.
#[test]
fn colour_off_keeps_every_word() {
    let line = "fn main() { let name = \"saya\"; // the entry";
    let plain = spans(line, Some(super::Language::Rust), Scheme::NoColour);
    insta::assert_debug_snapshot!("no_colour_runs", runs(line, Some(super::Language::Rust)));
    for span in &plain {
        assert_eq!(span.style, style(Token::Code, Scheme::NoColour), "no hue");
    }
    assert_eq!(
        joined(&plain),
        line,
        "colour going away changes no word of the line"
    );
}

/// The same for every language the table knows: the runs join back to the line
/// exactly, so a highlight can never lose or reorder a character.
#[test]
fn every_language_gives_the_line_back_unchanged() {
    let lines = [
        "SELECT id FROM t WHERE name = 'x' -- note",
        "def f(x): return x # note",
        "{\"a\": 1}",
        "let x = 1; // note",
        "if [ -f x ]; then echo 'yes'; fi",
    ];
    let languages = ALL;
    for language in languages {
        for line in lines {
            let spans = spans(line, Some(language), Scheme::Dark);
            assert_eq!(joined(&spans), line, "{language:?}: {line}");
        }
    }
}

const ALL: [super::Language; 11] = [
    super::Language::Rust,
    super::Language::Python,
    super::Language::JavaScript,
    super::Language::TypeScript,
    super::Language::Sql,
    super::Language::Shell,
    super::Language::Json,
    super::Language::Toml,
    super::Language::Yaml,
    super::Language::Go,
    super::Language::Java,
];

fn joined(spans: &[Span<'static>]) -> String {
    spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

/// A keyword is matched whole: `format` is not `for`, and a comment swallows
/// what follows it.
#[test]
fn words_are_matched_whole() {
    let kinds: Vec<Kind> = runs("format the for walk", Some(super::Language::Rust))
        .into_iter()
        .filter(|run| run.kind != Kind::Plain)
        .map(|run| run.kind)
        .collect();
    assert_eq!(kinds, vec![Kind::Keyword], "only `for` is reserved, whole");

    let runs = runs("let x = 1; // for the record", Some(super::Language::Rust));
    let last = runs.last().expect("a comment ends the line");
    assert_eq!(last.kind, Kind::Comment);
    assert_eq!(last.text, "// for the record");
}

/// The fence state: the opening word names the language, an unknown word leaves
/// the block plain, and the closing line is a marker row of its own.
#[test]
fn a_fence_carries_the_language_it_names() {
    let mut fence = Fence::default();
    assert_eq!(fence.toggle("```rust"), Some(true));
    assert!(fence.open());
    assert_eq!(fence.language(), Some(super::Language::Rust));
    assert_eq!(fence.toggle("```"), Some(false));
    assert!(!fence.open());
    assert_eq!(fence.language(), None);

    assert_eq!(fence.toggle("```cobol"), Some(true));
    assert_eq!(fence.language(), None, "an unknown word leaves it plain");
    assert_eq!(
        fence.toggle("plain text"),
        None,
        "an answer line is not a fence"
    );
}
