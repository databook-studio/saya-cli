//! The session deny list: a user-stated refusal of bare program names,
//! session-wide, evaluated deny-first at every door that execs a program by
//! name — `run_command`, `run_program`, the interpreter door.
//!
//! Deny is launch-only (`--deny`, user-layer `[session_commands] deny`) —
//! never mid-session — because a mid-session deny over a held grant would
//! leave a journaled token that gates nothing, and doing it honestly needs
//! grant revocation the session does not have. Deny therefore precedes every
//! grant by construction, not by ordering discipline.
//!
//! Deny bounds only the direct ask: a denied `curl` does not stop an allowed
//! `make` from invoking curl. The refusal says so, or the list becomes its
//! own comforting fiction.

use std::collections::BTreeSet;

/// The session's deny list: bare program names, sorted and deduped.
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionDeny {
    programs: BTreeSet<String>,
}

impl SessionDeny {
    /// Builds the list, validating every entry: a deny entry is a bare name
    /// (`is_bare_name`) — never a path, traversal, prefix, or glob. Prefix
    /// and glob denies under-block while reading stronger than they are.
    pub(crate) fn from_names(names: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut programs = BTreeSet::new();
        for name in names {
            validate_deny_entry(&name)?;
            programs.insert(name);
        }
        Ok(Self { programs })
    }

    /// The denied programs in sorted order — the journal's start-event
    /// payload and the status line's fact.
    pub(crate) fn programs(&self) -> Vec<String> {
        self.programs.iter().cloned().collect()
    }

    /// Whether `program` is denied. Deny is lane-blind by design: the user's
    /// unit of intent is the program, not the lane — "never is cheap to
    /// honor everywhere, and lane-selective honor would make the user's words
    /// mean less than they typed." The `door` rides along for the journal's
    /// per-firing payload, not for the decision.
    pub(crate) fn contains(&self, program: &str) -> bool {
        self.programs.contains(program)
    }
}

/// Validates one deny entry: a bare name — never a path, traversal,
/// prefix, or glob. A prefix deny (`cargo test`) under-blocks while reading
/// stronger than it is; a glob deny (`cur*`) is the same fiction in the
/// comforting direction, refused here rather than relied on the name shape
/// alone (`*` is otherwise a bare-shape character).
pub(crate) fn validate_deny_entry(name: &str) -> Result<(), String> {
    if !saya_types::is_bare_name(name) || name.contains('*') || name.contains('?') {
        return Err(format!(
            "deny entry `{name}` must be a bare program name — never a path, traversal, \
             prefix, or glob"
        ));
    }
    if name.contains(' ') {
        return Err(format!(
            "deny entry `{name}` must be a bare program name — never a path, traversal, \
             prefix, or glob"
        ));
    }
    Ok(())
}

/// The program name a tool call asks to exec, when the call names one: the
/// `program` string of `run_command` / `run_program` (the interpreter door
/// rides `run_program`'s refused names, so the same argument carries it).
pub(crate) fn call_program(tool: &str, arguments: &serde_json::Value) -> Option<String> {
    if !matches!(tool, "run_command" | "run_program") {
        return None;
    }
    arguments
        .get("program")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// The door a tool call execs through, for the journal's per-firing payload:
/// the tool's own name. The interpreter door rides `run_program` and journals
/// as `run_program` — the journal names the door the ask entered, not the
/// family the name belongs to.
pub(crate) fn call_door(tool: &str) -> &'static str {
    match tool {
        "run_command" => "run_command",
        "run_program" => "run_program",
        _ => "other",
    }
}

/// The typed refusal the model relays — pinned bytes, one builder, all
/// doors, all modes. It names the policy and where to change it, disclaims
/// "command not found", and carries the not-bounded clause: not a bug, and
/// not reassurance either.
pub(crate) fn denied_refusal(program: &str) -> String {
    format!(
        "refused: {program} is on this session's deny list — stated at launch or in \
         user config; this is saya's refusal, not a program failure. The deny list \
         bounds only the program named in the ask; allowed programs may still invoke it."
    )
}

/// The `/allow` parse refusal for a token whose payload names a denied
/// program: a grant cannot override the deny list (the lying-scope rule —
/// a token the door refuses would gate nothing).
pub(crate) fn allow_of_denied_refusal(token: &str) -> String {
    format!(
        "scope `{token}` is denied for this session; a grant cannot override the deny \
         list. Re-issue /allow without it."
    )
}

/// The launch contradiction: `--allow command:x` together with `--deny x`.
/// A statement that grants what it refuses is an exit-2 usage error.
pub(crate) fn launch_contradiction(name: &str) -> String {
    format!(
        "cannot both grant and deny `{name}`: `--allow command:{name}` states a grant the \
         `--deny {name}` refuses — relaunch granting or denying it, not both"
    )
}

/// The run-surface refusal for `--deny`: deny is session-shaped, and a run's
/// programs are pre-declared scopes. Its own pinned wording, in the permanent
/// refusal's class.
pub(crate) fn run_surface_refusal() -> String {
    "`--deny` is not available on runs, by design: deny is session-shaped, and a run's \
     programs are pre-declared scopes. Re-run without it."
        .to_owned()
}

/// The `ask`-surface refusal for `--deny`: the lane is
/// interactive-session-shaped, and refusal-only composes nothing.
pub(crate) fn ask_surface_refusal() -> String {
    "`--deny` states the interactive session's deny list: `saya ask` is a one-shot \
     question with no session state, so the flag has no universe there — launch the \
     interactive session (`saya --deny <program>`) instead"
        .to_owned()
}
