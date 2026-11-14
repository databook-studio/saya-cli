//! The bounds one `web_search` call runs under. Every one is a constant the
//! schema advertises, so what the model reads and what the tool enforces are
//! the same figures.

/// The query string's byte ceiling. Long enough for a real search phrase,
/// short enough that a query is never the reason a request or a result is
/// oversized.
pub(crate) const MAX_QUERY_BYTES: usize = 512;

/// The results a call that states none gets: enough to answer a question,
/// few enough that the snippets stay readable in one turn.
pub(crate) const DEFAULT_MAX_RESULTS: usize = 5;

/// The most results a call may ask for. The backend may return more; the tool
/// keeps this many, so the answer's size is the call's own bound.
pub(crate) const MAX_RESULTS: usize = 10;

/// The most characters one result's title may carry before it is cut.
pub(crate) const MAX_TITLE_CHARS: usize = 200;

/// The most characters one result's snippet may carry before it is cut. A
/// search snippet is a sentence or two; a backend that answers with a whole
/// page is cut rather than trusted to be short.
pub(crate) const MAX_SNIPPET_CHARS: usize = 300;

/// The byte ceiling on the whole rendered answer — every kept result's title,
/// URL, and snippet together. The lane's own cap is far above this; the bound
/// exists so a hostile backend answering with ten enormous results still
/// returns a bounded block with the elision marker inside it.
pub(crate) const MAX_RESULT_BYTES: usize = 16 * 1024;

/// The most domains one filter may name.
pub(crate) const MAX_DOMAINS: usize = 10;

/// The most bytes one domain may carry: the DNS name ceiling (RFC 1035), so a
/// filter entry that could never be a host is refused rather than matched
/// against nothing.
pub(crate) const MAX_DOMAIN_BYTES: usize = 253;
