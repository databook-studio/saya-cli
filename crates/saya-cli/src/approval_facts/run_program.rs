//! `run_program`'s fact lines: the containment, then the argv. The
//! interpreter door additionally carries the no-euphemism warning — the
//! session's own clause, the running platform's process-fork fact (U8:
//! per platform, never a stronger one). A fact the composition does not
//! carry produces no line; the program, its argv, and the typed-argv
//! contract are the call's own facts and always render.

use serde_json::Value;

use super::{ApprovalFacts, RunnerFacts, body};
use saya_types::is_refused_runner_program;

/// The call's typed argv, as the call states it — one element per argument,
/// displayed as the runner will pass it. Interior whitespace is collapsed so
/// a hostile element cannot split the line.
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

/// The allowlist membership line — which door the call enters, in the
/// battery's own order (`refuse::validate_call`: the name refusal before the
/// allowlist, the interpreter door only for names the runner refuses and the
/// interpreter scope explicitly carries).
fn membership_line(program: &str, facts: &RunnerFacts) -> (&'static str, String) {
    let in_interpreter = facts
        .interpreter_programs
        .iter()
        .any(|allowed| allowed == program);
    let in_runner = facts
        .runner_programs
        .iter()
        .any(|allowed| allowed == program);
    if is_refused_runner_program(program) {
        if in_interpreter {
            (
                "interpreter",
                format!("  program: {program} — staged in [jobs.interpreter] allow"),
            )
        } else {
            (
                "",
                format!(
                    "  program: {program} — refused by name: shells and interpreters are refused — stage it in `[jobs.interpreter] allow`"
                ),
            )
        }
    } else if in_runner {
        (
            "runner",
            format!("  program: {program} — allowlisted in [jobs.runner] allow"),
        )
    } else {
        ("", format!("  program: {program} — not allowlisted"))
    }
}

/// The sandbox line: the composed spawn's roots, egress, and pinned cwd.
fn sandbox_line(facts: &RunnerFacts) -> String {
    let roots = facts
        .fs_roots
        .iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let egress = if facts.net_allow.is_empty() {
        "none".to_string()
    } else {
        facts
            .net_allow
            .iter()
            .map(|(host, port)| format!("{host}:{port}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let pinned = facts
        .fs_roots
        .first()
        .map(|root| root.display().to_string())
        .unwrap_or_else(|| "the first root".to_string());
    format!(
        "  sandbox: reads/writes/exec bounded to {roots} · egress: {egress} · cwd pinned to {pinned}"
    )
}

pub(super) fn facts(
    arguments: &Value,
    facts: &ApprovalFacts,
    session_line: Option<String>,
) -> Option<String> {
    let program = arguments.get("program").and_then(Value::as_str)?;
    let mut lines = Vec::new();
    let (door, membership) = facts
        .runner
        .as_ref()
        .map(|runner| membership_line(program, runner))
        .unwrap_or(("", format!("  program: {program}")));
    if door == "interpreter" {
        lines.push(format!("run_program — {program} (interpreter)"));
    } else {
        lines.push(format!("run_program — {program}"));
    }
    lines.push(membership);
    lines.push(argv_line(program, arguments));
    lines.push("  typed argv: no shell, no interpolation, one element per argument".to_string());
    if let Some(runner) = facts.runner.as_ref() {
        lines.push(sandbox_line(runner));
        if facts.host_ran {
            lines.push(
                "  a host command ran in this session; staged program integrity is outside saya's control"
                    .to_string(),
            );
        }
        if runner.timeout_seconds > 0 {
            lines.push(format!(
                "  timeout: {}s — the call may narrow it, never widen it",
                runner.timeout_seconds
            ));
        }
        lines.push(if runner.credentials_declared == 0 {
            "  credential injection: none declared".to_string()
        } else {
            format!(
                "  credential injection: {} declared value(s) reach the child's environment",
                runner.credentials_declared
            )
        });
        lines.push(format!(
            "  output: capped at {} bytes per stream · redacted before it reaches the model",
            saya_harness::runner::OUTPUT_CAP_BYTES
        ));
        if door == "interpreter" {
            lines.push(format!(
                "  {}",
                crate::approval_text::interpreter_warning(
                    "session",
                    &[program.to_string()],
                    crate::interactive::session_activation::SESSION_FORK_FACT,
                )
            ));
        }
    }
    if let Some(session_line) = session_line {
        lines.push(session_line);
    }
    let header = lines.remove(0);
    Some(body(header, lines))
}
