//! `web_search`: one bounded query of the **configured** search backend, read
//! as bounded results.
//!
//! Saya runs no search service of its own, so this tool exists only where a
//! trusted layer configured one (`[web_search]`, in `saya-config`): with no
//! backend resolved, no surface advertises `web_search` and no request exists
//! to make. That is the whole of the off-by-default rule, and it is stated
//! once, in [`super::WebTools::compose`].
//!
//! The tool is `web_fetch`'s sibling on every other axis, and shares its lane:
//!
//! - **The same egress.** The request runs through the same harness fetch loop
//!   over the same job-wide transport, so the policy's HTTPS gate, the
//!   resolve-then-deny check, the wall clock and the byte bounds are the
//!   session's, unchanged. The one difference is the redirect bound: this
//!   lane's client follows **no** hop, because a search has exactly one
//!   destination — the endpoint the user configured ([`tool::limits`]).
//! - **The same consent.** [`tool::WebSearchTool`] is the rule `web_fetch`
//!   and the `shell` follow: `bypass` searches, `ask` puts each query
//!   to the operator once per call, a headless `ask` job is refused with the
//!   reason naming `--approval-mode bypass`, and the two no-egress modes
//!   advertise nothing.
//! - **The same delivery.** Results are untrusted text and reach the model the
//!   only way they may: as a [`ContextBlock`](saya_agent::ContextBlock) inside
//!   the same honest envelope a fetched page uses
//!   ([`bounded_block_envelope`](saya_harness::fetch::bounded_block_envelope)),
//!   so an unclosed block is never emitted.
//!
//! What is this tool's own is the *shape of a search*: a query, how many
//! results, and two domain filters ([`query`]); the backend's answer read into
//! bounded hits ([`parse`]); the answer rendered as numbered results
//! ([`render`]); and the bounds all three run under ([`bounds`]).

mod bounds;
mod definition;
mod parse;
mod query;
mod render;
mod tool;

#[cfg(test)]
mod tests;

pub(crate) use definition::{definition, is_web_search};
pub(crate) use tool::{WebSearchTool, limits, prompt};
