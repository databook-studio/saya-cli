//! The grant token suggestion: the bridge from one tool call to the
//! narrowest `--allow` grammar token a session grant for it would record.
//!
//! The suggestion is the only place that names what an interactive session
//! grant could be, and every token it produces is a word the existing
//! `/allow` grammar accepts — the parser (crate::commands::run::scopes) is
//! the grammar's authority, never a duplicate. `None` is a deliberate
//! answer, not a gap: it means the tool keeps asking every call.
//!
//! The host spelling for `fetch:` is the run engine's: the engine's fetch
//! policy compares `FetchDestination::new(url.scheme(), url.host_str())`,
//! and the shape is judged by the grammar's own `Destination` rule, so a
//! granted token names the destination the engine would fetch.

use crate::connection::ConnectionRegistry;
use saya_types::{Destination, is_bare_name, is_name_shaped, is_refused_runner_program};
use serde_json::Value;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The read-shaped SQL family: the tools whose call is one parsed
/// read-only statement against the named connection — keyed on the effect
/// fields (`external_side_effect: false`, `requires_approval: true`,
/// `local_state: None`) plus the `sql` + optional `connection` arguments,
/// not on which block of the toolset builder they sit in. One token,
/// `sql:<connection>`, covers every member on that connection. The
/// suggester's tests derive this list from the definitions themselves, so
/// a tool whose declared shape changes breaks the test rather than silently
/// joining or leaving the family.
///
/// Not in the family, by design: `bounded_sql_query_all` (its referent can
/// grow after approval — `/connect` mid-session would put a database inside
/// a grant made before it existed), `render_chart` (`external_side_effect:
/// true` — it writes a file and opens a browser, which the token's words do
/// not say), and everything with `requires_approval: false` (never asked,
/// so nothing to grant).
pub(crate) const SQL_FAMILY: &[&str] = &[
    "bounded_sql_query",
    "result_shape",
    "column_health",
    "join_check",
];

/// The turn's primary connection name, shared between the turn and the
/// approval deciders. The deciders are built before the turn's registry
/// exists (`prepare_turn` runs inside the turn), so the session universe
/// holds one of these and the turn binds it from the registry it builds;
/// the suggester reads it at ask time. Bound per turn — a mid-session
/// `/connect` names the next turn's primary, never a stale one. Unbound
/// suggests no token: fail closed, never a guessed name.
#[derive(Clone, Default)]
pub(crate) struct TurnPrimary(Arc<Mutex<Option<String>>>);

impl TurnPrimary {
    /// Binds the registry's primary — its real registry name, or nothing
    /// when no primary resolves.
    pub(crate) fn bind(&self, registry: &ConnectionRegistry) {
        *self.locked() = registry.primary().map(str::to_owned);
    }

    /// The bound primary, when the turn has bound one.
    pub(crate) fn get(&self) -> Option<String> {
        self.locked().clone()
    }

    /// The grant set is a plain cell, so a panicked holder cannot have left
    /// it in a state recovery must fear; taking the guard anyway keeps
    /// suggesting rather than wedging the session on a poisoned lock (the
    /// engine's own rule for its grant store).
    fn locked(&self) -> MutexGuard<'_, Option<String>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The narrowest `--allow` grammar token a session grant for this tool call
/// would record, or `None` when this slice does not grant the tool (which
/// means it keeps asking every call). `primary` is the turn's primary
/// connection, bound by the turn that owns the decider; the SQL family
/// names it when the call itself names no connection.
pub(crate) fn grant_token(tool: &str, arguments: &Value, primary: Option<&str>) -> Option<String> {
    match tool {
        "workspace_write" => Some("workspace-write".to_owned()),
        "scratch_sql" => Some("scratch".to_owned()),
        "http_fetch" | "http_download" => fetch_token(arguments),
        "run_program" => runner_token(arguments),
        name if SQL_FAMILY.contains(&name) => sql_token(arguments, primary),
        // Every other tool — the fan-out, render_chart, the never-asked
        // read tools — keeps asking.
        _ => None,
    }
}

/// `sql:<connection>` from the call's `connection` argument, or — when the
/// call names no connection (absent or empty, the registry's own
/// "primary" spelling) — the turn's primary, named by its real registry
/// name so `/grants` shows the database the user actually approved. The
/// payload is judged by the same name-shape rule `endpoint:` payloads
/// are; a name that fails it yields `None` — never a guessed token.
fn sql_token(arguments: &Value, primary: Option<&str>) -> Option<String> {
    let connection = match arguments.get("connection") {
        // Absent or empty — the registry's own "primary" spellings.
        None => primary?,
        Some(Value::String(name)) if name.is_empty() => primary?,
        // A named connection is the token's payload.
        Some(Value::String(name)) => name,
        // A malformed call names nothing: never a guessed token.
        Some(_) => return None,
    };
    if !is_name_shaped(connection) {
        return None;
    }
    Some(format!("sql:{connection}"))
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
