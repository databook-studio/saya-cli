//! The fetch family's fact lines: `http_fetch`'s URL, the destination token
//! a grant would record, and the untrusted-block lane; `http_download`'s
//! URL, target path, and the *remaining* download budget, read live from the
//! shared wallet.

use serde_json::Value;

use super::{ApprovalFacts, FetchFacts, body};

pub(super) fn facts(
    name: &str,
    arguments: &Value,
    grant: Option<&str>,
    facts: &ApprovalFacts,
    session_line: Option<String>,
) -> Option<String> {
    let (header, mut lines) = match name {
        "http_fetch" => fetch_lines(arguments, grant, facts.fetch.as_ref())?,
        _ => download_lines(arguments, facts.fetch.as_ref())?,
    };
    if let Some(session_line) = session_line {
        lines.push(session_line);
    }
    Some(body(header, lines))
}

/// `http_fetch`'s fact lines: the URL, the destination token a grant would
/// record, and the composed policy's structural refusals and bounds. Without
/// a composed member only the call's own lines render.
fn fetch_lines(
    arguments: &Value,
    grant: Option<&str>,
    fetch: Option<&FetchFacts>,
) -> Option<(String, Vec<String>)> {
    let url = arguments.get("url").and_then(Value::as_str)?;
    let mut lines = vec![format!(
        "  url: {}",
        crate::agent::tools::collapse_whitespace(url)
    )];
    if let Some(grant) = grant {
        lines.push(format!(
            "  destination a session grant would record: {grant}"
        ));
    }
    if let Some(fetch) = fetch {
        lines.push(
            "  https only · private, loopback, and link-local addresses are refused".to_string(),
        );
        lines.push(
            "  the body arrives as an escaped, labelled untrusted block — never raw bytes \
             into context"
                .to_string(),
        );
        lines.push(format!(
            "  bounds: body ≤ {} bytes · {}s wall clock · ≤ {} redirects",
            fetch.fetch_body_bytes, fetch.fetch_seconds, fetch.fetch_redirects,
        ));
    }
    Some(("http_fetch — external fetch".to_string(), lines))
}

/// `http_download`'s fact lines: the URL, the target path, and the
/// *remaining* download budget.
fn download_lines(arguments: &Value, fetch: Option<&FetchFacts>) -> Option<(String, Vec<String>)> {
    let url = arguments.get("url").and_then(Value::as_str)?;
    let destination = arguments.get("destination").and_then(Value::as_str)?;
    let mut lines = vec![
        format!("  url: {}", crate::agent::tools::collapse_whitespace(url)),
        format!(
            "  destination: {} — contained to the workspace root",
            crate::agent::tools::collapse_whitespace(destination)
        ),
    ];
    if let Some(wallet) = fetch.and_then(|fetch| fetch.download.as_ref()) {
        lines.push(format!(
            "  remaining download budget: {} bytes of {}",
            wallet.limit().saturating_sub(wallet.consumed()),
            wallet.limit(),
        ));
    }
    Some((
        "http_download — downloads into this session's workspace".to_string(),
        lines,
    ))
}
