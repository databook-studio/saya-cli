//! The bypass activation line: the one visible event when the mode takes
//! effect, in the product's no-euphemism register — the grammar's own word,
//! never "yolo", "danger", or a softened "auto". Emitted at launch, at
//! `/approvals bypass`, and re-printed on resume, always through the
//! existing notice/message paths, so every surface says the same words.
//!
//! The line composes the session's facts (DESIGN §1, §5): the mode's
//! meaning, the none-staged sentence, and the host lane's fact where the
//! lane composed.

use crate::interactive::session::state::SessionState;
use crate::interactive::session::universe::SessionUniverse;

/// The mode fact: what bypass is, and what it does not touch.
const BYPASS_ON: &str =
    "bypass on: every tool call runs without asking; every structural guard still applies.";

/// The host lane's fact: what the lane is when composed — unsandboxed, as
/// the user, their network, their filesystem. Stated on the bypass
/// activation line when the lane composed (DESIGN §5), so the moment of
/// choice carries the exposure, never a softened word.
pub(crate) const HOST_LANE_FACT: &str =
    "host commands run unsandboxed: as your user, your network, your filesystem.";

/// The interpreter fact (DESIGN §5): the mode's honesty about what it does
/// *not* open. Nothing stages interpreters — `[jobs.interpreter]` is not a
/// setting, and no other setting stages one — so this is the only sentence
/// the line can carry.
pub(crate) const NO_INTERPRETERS_STAGED: &str =
    "no interpreters are staged, so interpreter calls still refuse.";

/// The activation line for a bypass session: the mode fact, then the
/// none-staged sentence. One line per fact: the none-staged sentence starts
/// its own line, so the mode fact's sentence ends and the design's sentence
/// begins — a space joins them into "applies. no interpreters", a run-on with
/// a lowercase word starting a sentence.
/// Both facts keep the design's bytes.
///
/// The host lane's fact rides when the lane composed (DESIGN §5): host
/// commands run unsandboxed — as the user, their network, their filesystem.
/// The denied-names segment rides when the list is non-empty; an empty list
/// keeps today's bytes — no segment at all.
pub(crate) fn bypass_line(host_composed: bool, denied: &[String]) -> String {
    let mut line = String::from(BYPASS_ON);
    line.push('\n');
    line.push_str(NO_INTERPRETERS_STAGED);
    if host_composed {
        line.push('\n');
        line.push_str(HOST_LANE_FACT);
    }
    let segment = denied_segment(denied);
    if !segment.is_empty() {
        line.push('\n');
        line.push_str(&segment);
    }
    line
}

/// The denied-names segment of the activation line: the names the session
/// refuses in every mode, bypass included. Empty when the list is empty —
/// an empty list keeps today's bytes.
pub(crate) fn denied_segment(denied: &[String]) -> String {
    if denied.is_empty() {
        return String::new();
    }
    format!("denied for this session: {}", denied.join(", "))
}

/// Whether the session's mode parses to bypass — the one decision every
/// emission site (launch, `/approvals bypass`, resume) consults, so the
/// three cannot drift. An unparseable mode is never bypass.
pub(crate) fn is_bypass_mode(state: &SessionState) -> bool {
    state.approval_mode.parse() == Ok(saya_agent::ApprovalPolicy::Bypass)
}

/// Whether this launch itself activates bypass — a property of the system,
/// not a preference: the journal records a consent, so it records exactly
/// the launches where the user consented to the mode now. A fresh session
/// runs under the mode its launch stated, so bypass there is an activation;
/// a resume whose `--approval-mode` explicitly overrode the persisted mode
/// is a new statement of consent this process made; a resume that merely
/// carries the persisted mode re-prints the activation line for the user
/// (the mode is real again) but consents to nothing new — the record is
/// what made it operative — so nothing is journalled.
pub(crate) fn bypass_activated_at_launch(
    fresh: bool,
    mode_explicitly_stated: bool,
    mode: &str,
) -> bool {
    (fresh || mode_explicitly_stated) && mode.parse() == Ok(saya_agent::ApprovalPolicy::Bypass)
}

/// Whether a mid-session `/approvals bypass` newly activates the mode: the
/// mode was not bypass before the command and is bypass after it. A
/// re-statement over an already-bypass session changes nothing — like
/// `/allow` over an already-granted token, it says so but records no new
/// consent — so nothing is journalled for it.
pub(crate) fn bypass_activated_by_command(before: &str, after: &str) -> bool {
    before.parse() != Ok(saya_agent::ApprovalPolicy::Bypass)
        && after.parse() == Ok(saya_agent::ApprovalPolicy::Bypass)
}

/// The activation line when the session's mode is bypass, `None` otherwise —
/// the one call every emission site makes (launch, `/approvals bypass`,
/// resume), so the three surfaces cannot drift into different words.
pub(crate) fn line_if_bypass(state: &SessionState, universe: &SessionUniverse) -> Option<String> {
    is_bypass_mode(state).then(|| bypass_line(universe.host_composed(), &universe.deny_programs()))
}

#[cfg(test)]
#[path = "activation_tests.rs"]
mod tests;
