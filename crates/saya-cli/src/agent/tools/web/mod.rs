//! The web lane: `web_fetch`, one bounded GET of an https page, and
//! `web_search`, one bounded query of a search backend the user configured.
//! Neither is a second HTTP client — both drive the harness's own
//! [`HttpFetchTool`](saya_harness::fetch::HttpFetchTool), the same one the
//! interactive session's `http_fetch` is built from, so the egress policy, the
//! resolve-then-deny check, the per-hop redirect judgement, the wall clock and
//! the redirect bound are the session's, unchanged.
//!
//! [`tools`] is what a surface actually holds: the two tools behind one
//! advertisement, one dispatch, and one approval decision, so a surface cannot
//! offer a tool it could not serve. `web_fetch` exists wherever the mode
//! reaches a host; `web_search` only where a backend was also configured,
//! because Saya runs no search service of its own.
//!
//! Three things `web_fetch` adds, and nothing else:
//!
//! - **A body bound the call states.** The session's `http_fetch` refuses a
//!   body over its bound, because its destination is one context turn it must
//!   fill or fail. A page is not that: [`bounds`] takes a `max_bytes` the call
//!   narrows, and a longer page is cut head-and-tail with the elided count
//!   marked rather than refused.
//! - **A conversion.** [`text`] renders an HTML document as readable text;
//!   anything else is passed through byte for byte, because only the server's
//!   own `Content-Type` says what the bytes mean.
//! - **A consent decision.** [`tool`] is the rule the `shell` follows:
//!   `bypass` fetches, `ask` puts the URL to the operator once per
//!   call, a headless `ask` job is refused with the reason naming
//!   `--approval-mode bypass`, and the two no-egress modes advertise nothing.
//!   The operator, its serialisation, and the caller line are the shared seam
//!   ([`crate::agent::tools::consent`]).
//!
//! Fetched bytes are untrusted and reach the model the only way they may: as a
//! [`ContextBlock`](saya_agent::ContextBlock), rendered by
//! `saya_agent::render_untrusted_block` inside an honest JSON envelope — the
//! same lane `http_fetch` uses, through the same envelope rule
//! ([`bounded_block_envelope`](saya_harness::fetch::bounded_block_envelope)),
//! so an unclosed block is never emitted.

mod bounds;
mod definition;
mod egress;
mod entities;
#[cfg(test)]
pub(crate) mod fixture;
pub(crate) mod search;
mod text;
mod tool;
mod tools;

#[cfg(test)]
mod tests;

pub(crate) use definition::{definition as fetch_definition, is_web_fetch};
pub(crate) use egress::{WebEgress, egress};
pub(crate) use tool::{WebFetchTool, advertised, compose, prompt};
pub(crate) use tools::WebTools;

/// Whether `name` is one of the web lane's tools, whichever half owns it. The
/// one table a surface's dispatch, its approval routing, and its advertisement
/// all read, so they cannot disagree about what the lane offers.
pub(crate) fn is_web_tool(name: &str) -> bool {
    is_web_fetch(name) || search::is_web_search(name)
}
