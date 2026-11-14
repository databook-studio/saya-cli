//! One backend's answer, read into bounded results.
//!
//! The bytes are untrusted and the shape is the backend's, so this module is
//! the only place that knows either: a SearXNG answer carries its results at
//! the top level, Brave's under `web`. What both produce is the same
//! [`Hit`] — a title, a URL, and a snippet — already bounded, already
//! filtered, and already cut to what the call asked for.
//!
//! A malformed answer is a typed failure, never an empty result list: "the
//! endpoint answered with nothing" and "the endpoint did not answer with
//! results" are different facts, and only one of them means the query found
//! nothing.

use saya_agent::ToolError;
use saya_config::WebSearchBackend;
use serde_json::Value;

use super::bounds::{MAX_SNIPPET_CHARS, MAX_TITLE_CHARS};
use super::query::Search;
use crate::agent::tools::web::bounds::MAX_URL_BYTES;

/// One result, bounded: what the model reads, and nothing else from the
/// answer.
pub(crate) struct Hit {
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) snippet: String,
}

/// What one answer yielded: the hits to keep, and how many the call's own
/// domain filter removed.
pub(crate) struct Hits {
    pub(crate) hits: Vec<Hit>,
    pub(crate) filtered_out: usize,
}

/// Reads one answer, keeping at most `search.max_results` hits.
pub(crate) fn hits(
    backend: &WebSearchBackend,
    body: &str,
    search: &Search,
) -> Result<Hits, ToolError> {
    let answer: Value = serde_json::from_str(body)
        .map_err(|_| unreadable("the search endpoint did not answer with JSON"))?;
    let entries = results_array(backend, &answer)?;
    let mut filtered_out = 0usize;
    let mut hits = Vec::new();
    for entry in entries {
        let Some(hit) = hit(backend, entry) else {
            // An entry with no URL is not a result: there is nothing to cite
            // and nothing to open, so it is dropped rather than shown as a
            // title with no source.
            continue;
        };
        if !admitted(&hit.url, search) {
            filtered_out += 1;
            continue;
        }
        if hits.len() == search.max_results {
            // The call's bound is reached. Later entries are not read as
            // hits, and are not counted as filtered: the filter did not
            // remove them, the bound did.
            break;
        }
        hits.push(hit);
    }
    Ok(Hits { hits, filtered_out })
}

/// The results array, in this backend's own shape. A shape that is not the
/// one the backend documents is a typed failure: an answer with no results
/// array is not an answer with no results.
fn results_array<'a>(
    backend: &WebSearchBackend,
    answer: &'a Value,
) -> Result<&'a Vec<Value>, ToolError> {
    let array = match backend {
        WebSearchBackend::Searxng { .. } => answer.get("results"),
        WebSearchBackend::Brave { .. } => answer.get("web").and_then(|web| web.get("results")),
    };
    array
        .and_then(Value::as_array)
        .ok_or_else(|| unreadable("the search endpoint's answer carried no results list"))
}

/// One entry as a hit, or `None` where it is not one: no URL, or no object at
/// all.
fn hit(backend: &WebSearchBackend, entry: &Value) -> Option<Hit> {
    let url = entry.get("url")?.as_str()?;
    if url.is_empty() {
        return None;
    }
    let snippet = match backend {
        WebSearchBackend::Searxng { .. } => entry.get("content"),
        WebSearchBackend::Brave { .. } => entry.get("description"),
    };
    Some(Hit {
        title: bounded(text(entry.get("title")), MAX_TITLE_CHARS),
        url: bounded(url.to_owned(), MAX_URL_BYTES),
        snippet: bounded(text(snippet), MAX_SNIPPET_CHARS),
    })
}

/// A JSON string field as text; anything else — a number, an object, absence
/// — reads as empty rather than failing the whole answer.
fn text(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or_default().to_owned()
}

/// `text` cut to `max` characters (not bytes: a cut mid-character would be a
/// corrupt string, and a character is what a bound on prose means). A field cut
/// here is not the block's `truncated`: that flag is the whole answer's bound,
/// and a title shortened to its own is simply a shorter title.
fn bounded(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        return text;
    }
    text.chars().take(max).collect()
}

/// Whether a hit survives the call's domain filters. An allow list keeps only
/// what it names; a block list drops what it names; both are stated, so both
/// apply, the allow list first.
fn admitted(url: &str, search: &Search) -> bool {
    let host = host_of(url);
    if !search.allowed.is_empty() && !search.allowed.iter().any(|domain| covers(domain, &host)) {
        return false;
    }
    !search.blocked.iter().any(|domain| covers(domain, &host))
}

/// Whether `domain` is `host` or one of its parents: the match is on the
/// domain boundary, so `example.org` covers `blog.example.org` and never
/// `notexample.org`.
fn covers(domain: &str, host: &str) -> bool {
    host == domain
        || (host.len() > domain.len()
            && host.ends_with(domain)
            && host.as_bytes()[host.len() - domain.len() - 1] == b'.')
}

/// The host a result's URL names, lowercased. A reader, not a judge: the URL
/// is the backend's, and a URL with no host simply matches no domain.
fn host_of(url: &str) -> String {
    url.split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// One "the answer could not be read" failure, in the egress lane's own error
/// carrier.
fn unreadable(detail: &str) -> ToolError {
    ToolError::Fetch(detail.to_owned())
}
