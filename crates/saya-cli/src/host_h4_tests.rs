//! H4 — the prompt must not claim "no shell" when the program is a shell.
//!
//! Each test below names the property the slice exists for, in the order the
//! task lists them. Written before the fix; the failing output is pasted
//! verbatim into `REPORT.md` before any green edit.

use crate::approval_facts::{ApprovalFacts, HostFacts};
use serde_json::json;
use std::path::PathBuf;

/// The shell case's typed-argv line, pinned byte-exact: saya passes the
/// arguments without a shell of its own, and the program being approved is
/// itself a shell or interpreter, so what it receives is a script it will
/// interpret.
const SHELL_TYPED_ARGV_LINE: &str = "  typed argv: no shell of saya's own, one element per argument — the program is itself a shell or interpreter, unsandboxed like the lane, and it interprets what it receives: metacharacters, pipes, redirects and all";

/// The non-shell case keeps its current bytes exactly.
const PLAIN_TYPED_ARGV_LINE: &str =
    "  typed argv: no shell, no interpolation, one element per argument";

/// The fixed facts the host prompt states.
fn host_facts() -> ApprovalFacts {
    ApprovalFacts {
        host: Some(HostFacts {
            workspace_root: PathBuf::from("/home/user/proj"),
            timeout_seconds: 600,
            pass_env: Vec::new(),
        }),
        ..ApprovalFacts::default()
    }
}

fn body_for(program: &str, args: &[&str]) -> String {
    let arguments = json!({"program": program, "args": args});
    crate::approval_facts::call_facts(
        "run_command",
        &arguments,
        Some(format!("command:{program}").as_str()),
        &host_facts(),
        None,
        None,
    )
    .expect("the host lane states its own facts")
}

/// 1. A shell program must not carry the unqualified no-shell claim.
#[test]
fn a_shell_program_does_not_claim_no_shell() {
    let body = body_for("bash", &["-c", "echo hi"]);
    assert!(
        !body.contains(PLAIN_TYPED_ARGV_LINE),
        "a shell program must not claim the unqualified no-shell line: {body}"
    );
}

/// 2. Pinned, byte-exact: adding the shell case must not reword the common case.
#[test]
fn a_non_shell_program_keeps_its_typed_argv_bytes() {
    let body = body_for("npm", &["install"]);
    assert!(
        body.contains(PLAIN_TYPED_ARGV_LINE),
        "the non-shell line keeps its exact bytes: {body}"
    );
}

/// 3. The new wording's bytes, pinned.
#[test]
fn the_shell_line_says_the_argument_is_interpreted() {
    let body = body_for("bash", &["-c", "echo hi"]);
    assert!(
        body.contains(SHELL_TYPED_ARGV_LINE),
        "the shell line states the argv contract and the interpretation fact: {body}"
    );
}

/// Both lines present for a shell program: the new typed-argv line and the
/// existing interpreter-approval line.
#[test]
fn the_interpreter_warning_still_rides_alongside() {
    let body = body_for("bash", &["-c", "echo hi"]);
    assert!(
        body.contains(SHELL_TYPED_ARGV_LINE),
        "the shell typed-argv line rides: {body}"
    );
    assert!(
        body.contains("interpreter approval:"),
        "the existing interpreter-approval line still rides alongside: {body}"
    );
}

/// Extra property, and why it exists: the gate is the shared
/// `is_refused_runner_program` predicate, not a bash special-case. A branch
/// matching only `"bash"` would pass tests 1–4 and still reintroduce the
/// defect for every other shell and interpreter the warning gate already
/// names (`sh`, `zsh`, `python3`, …), letting the two lines drift apart again.
#[test]
fn every_refused_runner_program_gets_the_shell_line_not_just_bash() {
    for program in [
        "sh", "bash", "dash", "zsh", "fish", "python3", "node", "perl",
    ] {
        let body = body_for(program, &["-c", "echo hi"]);
        assert!(
            !body.contains(PLAIN_TYPED_ARGV_LINE),
            "{program} must not claim the unqualified no-shell line: {body}"
        );
        assert!(
            body.contains(SHELL_TYPED_ARGV_LINE),
            "{program} carries the shell typed-argv line: {body}"
        );
    }
}
