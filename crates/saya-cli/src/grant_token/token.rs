//! The per-family token builders: what each tool call's grant would record.

use super::SQL_FAMILY;
use crate::approval_facts::ApprovalFacts;
use saya_types::{Destination, is_bare_name, is_name_shaped, is_refused_runner_program};
use serde_json::Value;

/// The narrowest `--allow` grammar token a session grant for this tool call
/// would record, or `None` when this slice does not grant the tool (which
/// means it keeps asking every call). `primary` is the turn's primary
/// connection, bound by the turn that owns the decider; the SQL family
/// names it when the call itself names no connection. `facts` is the
/// surface's composition — a token is suggested only when the composition
/// carries what it names.
///
/// Failure contract, per family: an `interpreter:<p>` token is suggested
/// only when the composed `[jobs.interpreter] allow`
/// (`facts.runner.interpreter_programs`) holds `p`; a `runner:<p>` token
/// only when the composed `[jobs.runner] allow`
/// (`facts.runner.runner_programs`) holds `p`; `workspace-write` only when
/// a workspace root is bound; `scratch` only when a scratch member is
/// composed; `fetch:` only when a fetch member is composed; `command:`
/// only when the host lane is composed; `sql:` only for a call-named (or
/// primary-bound) connection. Pinned both directions for the runner family
/// (`grant_token_tests.rs:665` unstaged never offered, `:696` runner
/// outside the allowlist never offered, `:766` staged still offered).
pub(crate) fn grant_token(
    tool: &str,
    arguments: &Value,
    primary: Option<&str>,
    facts: &ApprovalFacts,
) -> Option<String> {
    match tool {
        "workspace_write" | "workspace_edit" => facts
            .workspace_root
            .is_some()
            .then(|| "workspace-write".to_owned()),
        "scratch_sql" => facts.scratch.is_some().then(|| "scratch".to_owned()),
        "scratch_import" => (facts.scratch.is_some() && facts.workspace_root.is_some())
            .then(|| "scratch".to_owned()),
        "http_fetch" | "http_download" => facts
            .fetch
            .is_some()
            .then(|| fetch_token(arguments))
            .flatten(),
        "run_program" => runner_token(arguments, facts.runner.as_ref()),
        "run_command" => command_token(arguments, facts.host.as_ref()),
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

/// `command:<program>` for a `run_command` call: the bare name, shells
/// included — the `runner:`/`interpreter:` family mirror deliberately does
/// not apply here (`command:bash` is the token, and the warning rides the
/// prompt). Suggested only when the lane composed (`host`): a token for a
/// program the composition cannot carry would record a grant that silences
/// the ask and refuses at execution — the U8 defect in this lane's shape.
/// A program argument that is absent, non-string, or not a bare name (never
/// a path, traversal, or prefix) yields `None`.
fn command_token(
    arguments: &Value,
    host: Option<&crate::approval_facts::HostFacts>,
) -> Option<String> {
    let program = arguments.get("program")?.as_str()?;
    if !is_bare_name(program) {
        return None;
    }
    host?;
    Some(format!("command:{program}"))
}

/// `runner:<program>` for a program the composition's runner door carries
/// in `[jobs.runner] allow`, `interpreter:<program>` for one its interpreter
/// door carries in `[jobs.interpreter] allow` — the family rule is the
/// engine's own (mirrored at grammar-parse time), and the membership is the
/// composed door's, because a grant can only pre-answer a call the
/// composition would actually run: a token for a program outside the
/// composed doors would record a grant that silences the ask and refuses at
/// execution — every time, for the rest of the session. No composed runner
/// (`None`) — an unproven host, no workspace root, nothing staged — offers
/// nothing: no `run_program` call can succeed. A program argument that is
/// absent, non-string, or not a bare name (the runner's own rule: programs,
/// never paths) yields `None`.
fn runner_token(
    arguments: &Value,
    runner: Option<&crate::approval_facts::RunnerFacts>,
) -> Option<String> {
    let program = arguments.get("program")?.as_str()?;
    if !is_bare_name(program) {
        return None;
    }
    let runner = runner?;
    if is_refused_runner_program(program) {
        runner
            .interpreter_programs
            .iter()
            .any(|staged| staged == program)
            .then(|| format!("interpreter:{program}"))
    } else {
        runner
            .runner_programs
            .iter()
            .any(|allowed| allowed == program)
            .then(|| format!("runner:{program}"))
    }
}
