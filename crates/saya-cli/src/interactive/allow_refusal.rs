//! The `/allow` composition refusals: the reasons a scope that parses under
//! the grammar still gates nothing in *this* session — the composed doors
//! and members it falls outside of. The `NOT_YET_WIRED` list (`scopes.rs`)
//! refuses families a *surface* cannot consume; these refuse tokens a
//! *composition* cannot carry, so their reasons are this session's own
//! facts, read off the same `ApprovalFacts` the approval prompts state
//! (U8: a grant the composition cannot honour would pre-answer asks into
//! refusals and silence every ask that would say something is wrong —
//! refused at the seed, with the reason, never accepted and inert).

use crate::approval_facts::ApprovalFacts;
use crate::interactive::session_runner::RUNNER_GAP_REMEDY;

/// The refusal for a scope that parses under the grammar but gates nothing
/// in this session's composition, `None` when the composition carries it.
/// The wording mirrors the `NOT_YET_WIRED` register: name the token, state
/// the true reason, say what approving would do, and name the slash
/// surface's own next step.
pub(crate) fn composition_refusal(token: &str, facts: &ApprovalFacts) -> Option<String> {
    reason(token, facts).map(|reason| {
        format!(
            "scope `{token}` is not carried by this session's composition: {reason}. It \
             parses, but nothing this session composed would consume it, so approving it \
             would gate nothing. Re-issue /allow without it."
        )
    })
}

/// Why `token` gates nothing in this composition — the composed door or
/// member it falls outside of, in that family's own vocabulary. `None`
/// when the composition carries the token (or the family is not judged
/// here).
fn reason(token: &str, facts: &ApprovalFacts) -> Option<String> {
    match token {
        "workspace-write" if facts.workspace_root.is_none() => Some(
            "no workspace root is bound, so the write-shaped file tools are not \
             composed and no workspace_write call can occur"
                .to_owned(),
        ),
        "scratch" if facts.scratch.is_none() => Some(
            "this session composed no scratch database, so no scratch_sql call can \
             occur"
                .to_owned(),
        ),
        _ if token.starts_with("fetch:") && facts.fetch.is_none() => Some(
            "this session composed no fetch member, so no http_fetch call can \
             occur"
                .to_owned(),
        ),
        _ if token.starts_with("runner:") || token.starts_with("interpreter:") => {
            runner_family_reason(token, facts)
        }
        // The host lane's own gate: a `command:` token parses under the
        // grammar on the session surface, and gates something only when the
        // lane composed. With the lane off — no workspace root bound — the
        // refusal states the workspace-root fact, in this surface-aware
        // refusal register.
        _ if token.starts_with("command:") && facts.host.is_none() => Some(
            "the host-command lane is not composed in this session, so the token gates \
             nothing in this session; no workspace root is bound, so the lane composed \
             nothing — bind one with `--workspace <dir>` or launch inside a worktree"
                .to_owned(),
        ),
        _ => None,
    }
}

/// The runner family's reasons, judged against the composed doors: no
/// composed runner means no `run_program` call can occur at all; a
/// composed runner carries only the programs its doors stage, and a token
/// outside them would grant a name every call refuses.
fn runner_family_reason(token: &str, facts: &ApprovalFacts) -> Option<String> {
    let Some(runner) = facts.runner.as_ref() else {
        return Some(format!(
            "this session composed no runner, so no run_program call can \
             occur — an unproven host or an unstaged config composes nothing, \
             and {RUNNER_GAP_REMEDY}"
        ));
    };
    if let Some(program) = token.strip_prefix("runner:") {
        return (!runner
            .runner_programs
            .iter()
            .any(|allowed| allowed == program))
        .then(|| {
            format!(
                "the composed [jobs.runner] allow carries no `{program}`, so the \
                     runner door refuses every call for it, and {RUNNER_GAP_REMEDY}"
            )
        });
    }
    let program = token.strip_prefix("interpreter:")?;
    (!runner
        .interpreter_programs
        .iter()
        .any(|staged| staged == program))
    .then(|| {
        format!(
            "[jobs.interpreter] allow staged no `{program}`, so the interpreter \
             door does not exist and an interpreter call for it refuses by name"
        )
    })
}
