//! The `suggest`-mode post-turn report — spec Phase 4b §2.
//!
//! After a completed `suggest` turn the runtime drains the observation log and
//! asks [`suggest_report`] what to surface. The report is the *evidence base*
//! a candidate would have been built from — not a fabricated proposal: the
//! model never wrote one, because `suggest` hides `contract_propose`.

use crate::agent::tools::{DrainedObservations, ObservationOutcome};

/// Builds the `suggest`-mode report from a turn's drained observations, or
/// `None` when the turn was not proposal-worthy (no succeeded read touched a
/// named object — there was nothing to propose from).
///
/// **Decision (spec §3, the five-second bound):** this is *not* a second
/// extraction pass over the turn. The observations were already collected
/// in-band during the tool calls the model made; `drain` is a bounded read of a
/// ≤32-entry buffer. There is no additional application-level work to bound, so
/// there is nothing to spawn and nothing the shutdown path must await. The
/// spec's "at most one extraction pass" is the observation recording itself,
/// which already happened within the turn. A detached task is not needed and
/// so not spawned — a detached task that outlives its turn is worse than a
/// missing bound, and there is no bound to miss here.
///
/// The report lists the objects a succeeded read touched (the evidence a
/// candidate would have been built from) and states plainly that nothing was
/// stored. It cannot list claim *text* the model never produced — `suggest`
/// hides `contract_propose`, so the model never wrote a proposal; the honest
/// report is the evidence base, not a fabricated proposal.
pub(crate) fn suggest_report(observations: &DrainedObservations) -> Option<String> {
    let touched: Vec<String> = proposal_worthy_objects(observations);
    if touched.is_empty() {
        return None;
    }
    let mut lines = Vec::with_capacity(touched.len() + 2);
    lines.push("Learning is in suggest mode: nothing was stored.".into());
    lines.push(
        "A successful read this turn touched these objects, which under \
        learning = auto-candidate could have seeded candidate claims for review:"
            .into(),
    );
    for qualified in &touched {
        lines.push(format!("  - {qualified}"));
    }
    lines.push(
        "No claims were written; confirm candidates yourself with \
        `saya contracts queue` after switching to auto-candidate."
            .into(),
    );
    Some(lines.join("\n"))
}

/// The distinct qualified objects a succeeded read observation named, in
/// first-seen order, joined as `catalog.schema.object` (trailing parts only
/// when the observation named fewer). A failed or denied query touched nothing
/// a proposal could lean on, matching `contract_propose`'s evidence rule.
fn proposal_worthy_objects(observations: &DrainedObservations) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for obs in &observations.observations {
        if obs.outcome != ObservationOutcome::Succeeded {
            continue;
        }
        for path in &obs.objects {
            if path.is_empty() {
                continue;
            }
            let qualified = path.join(".");
            if seen.insert(qualified.clone()) {
                out.push(qualified);
            }
        }
    }
    out
}
