//! The grant token suggestion: the bridge from one tool call to the
//! narrowest `--allow` grammar token a session grant for it would record.
//!
//! The suggestion is the only place that names what an interactive session
//! grant could be, and every token it produces is a word the existing
//! `--allow` parser accepts — the parser (crate::commands::run::scopes) is
//! the grammar's authority, never a duplicate. `None` is a deliberate
//! answer, not a gap: it means the tool keeps asking every call, which is
//! this slice's rule for the SQL tools (a separate design decision covers
//! them) and for anything whose arguments do not name a grantable shape.
//!
//! The host spelling for `fetch:` is the run engine's: the engine's fetch
//! policy compares `FetchDestination::new(url.scheme(), url.host_str())`,
//! and the shape is judged by the grammar's own `Destination` rule, so a
//! granted token names the destination the engine would fetch.

use saya_types::{Destination, is_bare_name, is_refused_runner_program};
use serde_json::Value;

/// The narrowest `--allow` grammar token a session grant for this tool call
/// would record, or `None` when this slice does not grant the tool (which
/// means it keeps asking every call — never an "allow").
pub(crate) fn grant_token(tool: &str, arguments: &Value) -> Option<String> {
    match tool {
        "workspace_write" => Some("workspace-write".to_owned()),
        "scratch_sql" => Some("scratch".to_owned()),
        "http_fetch" | "http_download" => fetch_token(arguments),
        "run_program" => runner_token(arguments),
        // Every other tool — every SQL tool included — keeps asking.
        _ => None,
    }
}

/// `fetch:<scheme>+<host>` from the call's URL argument, spelled the way the
/// run engine normalises a destination: the URL parser's scheme and host
/// (host only, no port, lowercased), judged by the grammar's `Destination`
/// rule. A URL that does not parse, names no host, or fails the grammar's
/// shape yields `None` — never a guessed token.
fn fetch_token(arguments: &Value) -> Option<String> {
    let url = arguments.get("url")?.as_str()?;
    let parsed = url::Url::parse(url).ok()?;
    let destination = Destination::new(
        parsed.scheme().to_ascii_lowercase(),
        parsed.host_str()?.to_ascii_lowercase(),
    )
    .ok()?;
    Some(format!("fetch:{}+{}", destination.scheme, destination.host))
}

/// `runner:<program>` for a program the run engine's runner can run,
/// `interpreter:<program>` for one it refuses by name — the engine's own
/// family rule, mirrored at grammar-parse time, not an invented one. A
/// program argument that is absent, non-string, or not a bare name (the
/// runner's own rule: programs, never paths) yields `None`.
fn runner_token(arguments: &Value) -> Option<String> {
    let program = arguments.get("program")?.as_str()?;
    if !is_bare_name(program) {
        return None;
    }
    let family = if is_refused_runner_program(program) {
        "interpreter"
    } else {
        "runner"
    };
    Some(format!("{family}:{program}"))
}

/// The answers line both approval frontends render: the third answer names
/// the token verbatim when one exists, and with none the line offers the two
/// answers and says so — it must never offer a grant it cannot name.
pub(crate) fn session_answers_line(grant: Option<&str>) -> String {
    match grant {
        Some(token) => format!("[a] allow once   [s] allow {token} for this session   [d] deny"),
        None => "[a] allow once   [d] deny   (no session grant for this tool)".to_owned(),
    }
}
