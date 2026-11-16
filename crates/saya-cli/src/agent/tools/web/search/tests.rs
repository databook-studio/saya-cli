//! The `web_search` battery, in four parts:
//!
//! - `advertisement` — what the schema says, that the tool exists only where
//!   a backend is configured and the mode reaches the network, and that
//!   `[tools] allow` can take it away.
//! - `consent` — the same consent rule `web_fetch` follows: who is asked, once
//!   per call, and what a call earns when nobody can be.
//! - `results` — the request the tool builds, the bounds it applies to what
//!   comes back, and the domain filters.
//! - `refusals` — what a call refuses, and what is a typed failure rather
//!   than a result.
//! - `secrets` — a configured credential reaches the wire and nowhere else.
//!
//! The wire is the hermetic [`fixture`](super::super::fixture): no test here
//! opens a socket.

/// A SearXNG answer, in the shape its JSON API returns — the wire the
/// battery's tests answer with unless they are testing a refusal.
fn searxng_answer() -> String {
    serde_json::json!({
        "query": "rust lifetimes",
        "results": [
            {"url": "https://doc.rust-lang.org/book/ch10-03.html", "title": "Validating References",
             "content": "Lifetimes ensure that references are valid."},
            {"url": "https://blog.example.org/lifetimes", "title": "Lifetimes, explained",
             "content": "A longer explanation."},
            {"url": "https://forum.example.net/t/42", "title": "Forum thread",
             "content": "A question."}
        ]
    })
    .to_string()
}

#[path = "tests/advertisement.rs"]
mod advertisement;
#[path = "tests/consent.rs"]
mod consent;
#[path = "tests/refusals.rs"]
mod refusals;
#[path = "tests/results.rs"]
mod results;
#[path = "tests/secrets.rs"]
mod secrets;
