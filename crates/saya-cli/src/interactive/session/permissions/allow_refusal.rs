//! The `/allow` composition refusals: the reasons a scope that parses under
//! the grammar still gates nothing in *this* session — the composed doors
//! and members it falls outside of. The `NOT_YET_WIRED` list (`scopes.rs`)
//! refuses families a *surface* cannot consume; these refuse tokens a
//! *composition* cannot carry, so their reasons are this session's own
//! facts, read off the same `ApprovalFacts` the approval prompts state: a
//! grant the composition cannot honour would pre-answer asks into refusals and
//! silence every ask that would say something is wrong, so it is refused at
//! the seed, with the reason, never accepted and inert.

use crate::approval_facts::ApprovalFacts;

/// The refusal the removed runner and interpreter grant families are
/// refused with, on every session surface: one wording for both families,
/// naming the replacement. A session composes no runner, so the tokens still
/// parse — a persisted grant or a muscle memory is not an unknown scope — but
/// a session refuses them.
pub(crate) const REMOVED_RUNNER_GRANT_REFUSAL: &str =
    "runner and interpreter grants were removed; use the shell tool";

/// The refusal for a scope that parses under the grammar but gates nothing
/// in this session's composition, `None` when the composition carries it.
/// The wording mirrors the `NOT_YET_WIRED` register: name the token, state
/// the true reason, say what approving would do, and name the slash
/// surface's own next step. A family the product does not carry is refused
/// in its own words — its absence is the reason, not a gap in this
/// composition.
pub(crate) fn composition_refusal(token: &str, facts: &ApprovalFacts) -> Option<String> {
    if token.starts_with("runner:") || token.starts_with("interpreter:") {
        return Some(format!(
            "scope `{token}`: {REMOVED_RUNNER_GRANT_REFUSAL}. It parses, but nothing \
             this session composed would consume it, so approving it would gate \
             nothing. Re-issue /allow without it."
        ));
    }
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
             composed and no write or edit call can occur"
                .to_owned(),
        ),
        _ if token.starts_with("fetch:") && facts.fetch.is_none() => Some(
            "this session composed no fetch member, so no http_fetch call can \
             occur"
                .to_owned(),
        ),
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

#[cfg(test)]
mod tests {
    use super::{REMOVED_RUNNER_GRANT_REFUSAL, composition_refusal};
    use crate::approval_facts::ApprovalFacts;

    /// The workspace-root refusal names both guarded file tools: the token
    /// gates the family, never one tool's name.
    #[test]
    fn the_workspace_refusal_names_both_write_shaped_tools() {
        let refusal = composition_refusal("workspace-write", &ApprovalFacts::default())
            .expect("no workspace root, the token refuses");
        assert!(
            refusal.contains("no write or edit call can occur"),
            "the refusal names both file tools: {refusal}"
        );
    }

    /// The absent families are refused whatever the composition carries: no
    /// fact can carry them back.
    #[test]
    fn the_removed_runner_families_are_refused_by_name() {
        for token in ["runner:report", "interpreter:python3"] {
            let refusal = composition_refusal(token, &ApprovalFacts::default())
                .expect("the removed families are refused");
            assert!(
                refusal.contains(REMOVED_RUNNER_GRANT_REFUSAL),
                "the refusal states the removal: {refusal}"
            );
            assert!(
                refusal.contains("use the shell tool"),
                "the refusal names the replacement: {refusal}"
            );
        }
    }
}
