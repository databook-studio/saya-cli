//! `/grants`: the store's tokens rendered under the mode's own words.

use saya_agent::{ApprovalPolicy, SessionGrants};

/// `/grants`: the store's tokens verbatim, one per line, sorted, under a
/// header stating the lifetime, with a count — and an explicit empty state,
/// never a bare nothing. The words are the record, and they are the same
/// words the prompt offered. Under bypass the mode is stated **first**:
/// a count alone would read "nothing runs", when the truth is that every
/// call runs without asking and the store is never consulted. Every other
/// mode renders today's bytes exactly — the listing is the store's, and the
/// mode is the engine's own (`SessionPolicy::mode`).
pub(crate) fn listing(mode: ApprovalPolicy, grants: &SessionGrants) -> String {
    let mut out = String::new();
    if mode == ApprovalPolicy::Bypass {
        out.push_str("mode bypass: every call runs without asking; grants are not consulted\n");
    }
    let tokens = grants.tokens();
    out.push_str(&format!(
        "session grants (die with this session): {}",
        tokens.len()
    ));
    if tokens.is_empty() {
        out.push_str(
            "\n  (none — nothing pre-answers this session yet; /allow <scopes> \
             or answer [s] at an ask)",
        );
    } else {
        for token in tokens {
            out.push_str("\n  ");
            out.push_str(&token);
        }
    }
    out
}
