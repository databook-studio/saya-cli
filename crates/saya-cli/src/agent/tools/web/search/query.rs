//! One call's arguments, and the request they become.
//!
//! Everything the model states is read and bounded here, before a URL exists:
//! the query, how many results to keep, and the two domain filters. The URL
//! is then built from the **configured** endpoint plus those arguments, so
//! the request can only ever go to the host the user stated — the model
//! supplies words, never a destination.
//!
//! Domain filters are applied to the results rather than pushed to the
//! backend: the backends' own filter syntaxes differ and none of them is the
//! same rule, so one local rule is what the model is promised. The cost is
//! stated where the model can see it — the answer reports how many results
//! the filter removed.

use saya_agent::ToolError;
use saya_config::WebSearchBackend;
use serde_json::Value;

use super::bounds::{
    DEFAULT_MAX_RESULTS, MAX_DOMAIN_BYTES, MAX_DOMAINS, MAX_QUERY_BYTES, MAX_RESULTS,
};

/// One call's arguments, read and bounded.
pub(crate) struct Search {
    pub(crate) query: String,
    pub(crate) max_results: usize,
    pub(crate) allowed: Vec<String>,
    pub(crate) blocked: Vec<String>,
}

/// Reads one call's arguments. Every mistake is refused typed, naming the
/// argument and its bound, rather than clamped — a call never believes it
/// asked for something it did not.
pub(crate) fn read(arguments: &Value) -> Result<Search, ToolError> {
    let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
    for name in object.keys() {
        if !matches!(
            name.as_str(),
            "query" | "max_results" | "allowed_domains" | "blocked_domains"
        ) {
            return Err(ToolError::UnsupportedProperty);
        }
    }
    let query = object
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| refused("query must be a string"))?;
    if query.is_empty() || query.len() > MAX_QUERY_BYTES {
        return Err(refused(&format!(
            "query must be 1 to {MAX_QUERY_BYTES} bytes"
        )));
    }
    let max_results = match object.get("max_results") {
        None => DEFAULT_MAX_RESULTS,
        Some(value) => {
            let bound = value
                .as_u64()
                .ok_or_else(|| refused("max_results must be an integer"))?;
            let bound = usize::try_from(bound).unwrap_or(usize::MAX);
            if !(1..=MAX_RESULTS).contains(&bound) {
                return Err(refused(&format!(
                    "max_results must be between 1 and {MAX_RESULTS}"
                )));
            }
            bound
        }
    };
    Ok(Search {
        query: query.to_owned(),
        max_results,
        allowed: domains(object.get("allowed_domains"), "allowed_domains")?,
        blocked: domains(object.get("blocked_domains"), "blocked_domains")?,
    })
}

/// One domain filter: a bounded list of bare host names, lowercased so the
/// match is the DNS one (names are case-insensitive).
fn domains(value: Option<&Value>, name: &str) -> Result<Vec<String>, ToolError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let entries = value
        .as_array()
        .ok_or_else(|| refused(&format!("{name} must be an array of strings")))?;
    if entries.len() > MAX_DOMAINS {
        return Err(refused(&format!(
            "{name} must hold at most {MAX_DOMAINS} domains"
        )));
    }
    let mut domains = Vec::with_capacity(entries.len());
    for entry in entries {
        let domain = entry
            .as_str()
            .ok_or_else(|| refused(&format!("{name} must be an array of strings")))?;
        if !is_domain(domain) {
            return Err(refused(&format!(
                "{name} entry {domain:?} must be a bare domain name, not a URL or a path"
            )));
        }
        domains.push(domain.to_ascii_lowercase());
    }
    Ok(domains)
}

/// Whether an entry is a bare host name: letters, digits, hyphens, and dots,
/// within the DNS length bound. No scheme, no port, no path, no wildcard —
/// a filter that could never be a host is refused rather than matched against
/// nothing.
fn is_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= MAX_DOMAIN_BYTES
        && domain
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.')
}

/// The URL one call asks for: the **configured** endpoint plus this call's
/// query, encoded. Each backend's own JSON API is asked in its own shape.
pub(crate) fn url(backend: &WebSearchBackend, search: &Search) -> String {
    let endpoint = backend.endpoint();
    let separator = if endpoint.contains('?') { '&' } else { '?' };
    let query = encode(&search.query);
    match backend {
        // SearXNG's JSON API is a parameter on the instance's own search
        // endpoint; the instance decides which engines answer.
        WebSearchBackend::Searxng { .. } => {
            format!("{endpoint}{separator}q={query}&format=json")
        }
        // Brave's Web Search API, asked for exactly as many results as the
        // call stated — the tool keeps the same number, so the request never
        // asks the provider for more work than the answer uses.
        WebSearchBackend::Brave { .. } => {
            format!(
                "{endpoint}{separator}q={query}&count={}",
                search.max_results
            )
        }
    }
}

/// Percent-encodes one query as a URL query-string value: the unreserved
/// characters (RFC 3986 §2.3) pass through, everything else — spaces
/// included — is escaped byte by byte. Written here rather than reached for,
/// so the encoding is a stated rule rather than a dependency's default.
fn encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// One argument refusal, in the egress lane's own error carrier.
fn refused(detail: &str) -> ToolError {
    ToolError::Fetch(format!("invalid tool arguments: {detail}"))
}
