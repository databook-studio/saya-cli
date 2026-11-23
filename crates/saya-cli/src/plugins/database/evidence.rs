//! The evidence the database family is advertised from: what a request's own
//! words have to show before a run with a bound profile is offered the family.
//!
//! Deliberately plain and short. A false latch costs this session the family's
//! bytes and never an answer, while a missed one leaves the model the
//! `use_database` ask, so the rule reads for the words a person writes when they
//! mean a database — and nothing else.

use super::family::Bound;

/// The words a request may state, in the singular: a plural or a verb form
/// reaches them through [`states_word`]. `total` and `data` are here because
/// they are how a request asks for the database's own figures without naming
/// it — "what was our total last month", "pull the data behind that".
const WORK_WORDS: &[&str] = &[
    "database", "db", "sql", "schema", "table", "column", "query", "row", "count", "sum",
    "average", "revenue", "total", "data",
];

/// The phrases a request may state, side by side in this order: `how many` is
/// the question a person writes when they want a number out of a database, and
/// it names nothing the words above could catch — "how many orders are there".
const WORK_PHRASES: &[&[&str]] = &[&["how", "many"]];

/// The whole words of `text`, lowercased: punctuation is a boundary, so `table`
/// is not read out of `comfortable` and a qualified `public.orders` states
/// `orders`.
fn tokens(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// Whether `token` states one of the words above as itself or in a plural or
/// verb form: `databases`, `tables`, `queries`, `querying` and `counted` all do.
/// Only the endings those forms add are folded — `country` and `summary` are
/// themselves, so `count` and `sum` are never read out of a word that merely
/// starts with them.
fn states_word(token: &str) -> bool {
    if WORK_WORDS.contains(&token) {
        return true;
    }
    let mut forms: Vec<String> = Vec::new();
    if let Some(stem) = token.strip_suffix("ies")
        && !stem.is_empty()
    {
        forms.push(format!("{stem}y"));
    }
    for suffix in ["ing", "ed", "es", "s"] {
        if let Some(stem) = token.strip_suffix(suffix)
            && !stem.is_empty()
        {
            forms.push(stem.to_owned());
        }
    }
    forms.iter().any(|form| WORK_WORDS.contains(&form.as_str()))
}

/// Whether `tokens` states one of the phrases above, its words side by side.
fn states_phrase(tokens: &[String]) -> bool {
    WORK_PHRASES.iter().any(|phrase| {
        tokens.windows(phrase.len()).any(|window| {
            window
                .iter()
                .zip(*phrase)
                .all(|(token, word)| token == word)
        })
    })
}

/// Whether a request shows database work: one of its words, one of its
/// phrases, the two a SQL statement always uses together, or a name this run's
/// own schema holds — the connections' names, the engines they speak, and the
/// object names cached from their schema.
pub(super) fn shows_work(text: &str, bound: &Bound) -> bool {
    if bound.is_empty() {
        return false;
    }
    let tokens = tokens(text);
    if tokens.iter().any(|token| states_word(token)) {
        return true;
    }
    if states_phrase(&tokens) {
        return true;
    }
    // Prose does not use `select` and `from` together; a statement does.
    if tokens.iter().any(|token| token == "select") && tokens.iter().any(|token| token == "from") {
        return true;
    }
    let names = bound.evidence_names();
    tokens.iter().any(|token| names.contains(token))
}
