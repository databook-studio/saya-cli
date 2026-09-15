//! `run_command`'s fact lines: the absence said in the sandbox line's slot,
//! then the bounds the code applies. A shell- or interpreter-family name
//! additionally carries the no-euphemism warning with this lane's clause —
//! the model writes the program the interpreter runs, and there is no
//! sandbox; it runs as you.

use serde_json::Value;

use super::{ApprovalFacts, HostFacts, body};
use saya_types::is_refused_runner_program;

/// The call's typed argv, as the call states it — one element per argument,
/// displayed as the host lane will pass it. Interior whitespace is collapsed
/// so a hostile element cannot split the line; the journal carries verbatim
/// argv.
fn argv_line(program: &str, arguments: &Value) -> String {
    let args = arguments
        .get("args")
        .and_then(Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(Value::as_str)
                .map(crate::agent::tools::collapse_whitespace)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if args.is_empty() {
        format!("  argv: {program}")
    } else {
        format!("  argv: {program} {}", args.join(" "))
    }
}

/// The environment line: the built child's base names plus the configured
/// `pass_env` names — what the child demonstrably receives, and nothing
/// else.
fn environment_line(facts: &HostFacts) -> String {
    let mut names = vec!["PATH", "HOME", "TMPDIR"];
    let mut extra: Vec<&str> = facts
        .pass_env
        .iter()
        .map(String::as_str)
        .filter(|name| !matches!(*name, "PATH" | "HOME" | "TMPDIR"))
        .collect();
    extra.sort();
    names.extend(extra);
    if facts.pass_env.is_empty() {
        format!(
            "  environment: {} (built for the child; nothing else reaches it)",
            names.join(", ")
        )
    } else {
        format!(
            "  environment: {} (built for the child: PATH, HOME, TMPDIR plus configured pass_env {}; nothing else reaches it)",
            names.join(", "),
            facts.pass_env.join(", ")
        )
    }
}

pub(super) fn facts(
    arguments: &Value,
    facts: &ApprovalFacts,
    session_line: Option<String>,
) -> Option<String> {
    let host = facts.host.as_ref()?;
    let program = arguments.get("program").and_then(Value::as_str)?;
    let mut lines = Vec::new();
    lines.push(format!("run_command — {program}"));
    lines.push(format!(
        "  program: {program} — host command: resolved on your PATH, unsandboxed"
    ));
    lines.push(argv_line(program, arguments));
    if is_refused_runner_program(program) {
        lines.push(
            "  typed argv: no shell of saya's own, one element per argument — the program is itself a shell or interpreter, unsandboxed like the lane, and it interprets what it receives: metacharacters, pipes, redirects and all"
                .to_string(),
        );
    } else {
        lines
            .push("  typed argv: no shell, no interpolation, one element per argument".to_string());
    }
    lines.push(
        "  no sandbox: runs as your user — your whole filesystem, your network, unconfined"
            .to_string(),
    );
    lines.push(format!(
        "  cwd: pinned to {}",
        host.workspace_root.display()
    ));
    lines.push(environment_line(host));
    if host.timeout_seconds > 0 {
        lines.push(format!(
            "  timeout: {}s — the call may narrow it, never widen it",
            host.timeout_seconds
        ));
    }
    lines.push(format!(
        "  output: capped at {} bytes per stream · redacted before it reaches the model",
        saya_harness::runner::OUTPUT_CAP_BYTES
    ));
    lines.push(
        "  what the program spawns, downloads, or executes is not bounded by anything above"
            .to_string(),
    );
    if is_refused_runner_program(program) {
        lines.push(
            "  interpreter approval: the model writes the program the interpreter runs, and \
             there is no sandbox — it runs as you. What the interpreter computes is not a \
             reviewed, fixed binary."
                .to_string(),
        );
    }
    if let Some(session_line) = session_line {
        lines.push(session_line);
    }
    let header = lines.remove(0);
    Some(body(header, lines))
}
