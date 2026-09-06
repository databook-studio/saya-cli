//! Text shaping for [`saya_agent::AgentEvent::KnowledgeProposed`] — the line a
//! user sees when SAYA comes away from a turn knowing something it did not
//! know before.
//!
//! Until now neither adapter rendered this event: the TUI dropped it on its
//! catch-all arm and the headless renderer printed `unrecognized agent event`.
//! That was the worst place in the product to be silent — the whole pitch is
//! that memory accumulates without being managed, and the one moment that
//! proves it showed nothing, or an error string.
//!
//! Two wording constraints, both correctness rather than style:
//!
//! - **No id.** Learning is not something the user asked for, so the line must
//!   not hand them a 64-character hash to deal with. A candidate is acted on
//!   through `/queue`, which prints its own short prefix; a confirmed fact
//!   needs no handle at all.
//! - **Recorded, not believed.** A user-stated fact is *recorded*; an inferred
//!   one is *noted* and says plainly that it is waiting for review. Collapsing
//!   the two would let an inference the user never made read as their own word.

use saya_agent::ProposedClaimDto;
use saya_types::ClaimStatus;

/// Shapes the text block for one learned claim, for any adapter that prints it.
///
/// Returns an empty string when the claim cannot be described in the user's
/// vocabulary; callers may treat empty as "render nothing". Rendering never
/// fails a turn — a fact SAYA cannot phrase is still stored, and `contracts
/// list` will show it.
pub(crate) fn knowledge_learned_text(claim: &ProposedClaimDto) -> String {
    let phrase = describe(claim);
    if phrase.is_empty() {
        return String::new();
    }
    match claim.status {
        ClaimStatus::Confirmed => {
            format!("memory learned · {phrase} for {}\n", claim.object)
        }
        // Anything not confirmed is an inference, and says so. `ClaimStatus` is
        // `#[non_exhaustive]`: a future status reads as unreviewed, which is the
        // cautious mark.
        _ => format!(
            "memory noted · {phrase} for {} — unconfirmed, review with /queue\n",
            claim.object
        ),
    }
}

/// The fact in the user's words: the kind token turned into a phrase, the
/// value, and the column when the claim is column-scoped.
///
/// An unrecognised kind returns empty rather than printing the raw token — a
/// line reading `table_grain_v2 order_id` is noise, and the claim is still in
/// `contracts list` where the token belongs.
fn describe(claim: &ProposedClaimDto) -> String {
    let value = elide(&claim.value);
    match (claim.kind.as_str(), claim.column.as_deref()) {
        ("table_alias", _) => format!("the alias \"{value}\""),
        ("table_grain", _) => format!("the grain \"{value}\""),
        ("table_description", _) => format!("the description \"{value}\""),
        ("default_time_column", _) => format!("{value} as the default time column"),
        ("column_role", Some(column)) => format!("{column} as {value}"),
        ("column_description", Some(column)) => format!("{column} described as \"{value}\""),
        ("join_rule", _) => format!("the join rule \"{value}\""),
        ("metric_definition", _) => format!("the metric \"{value}\""),
        // An unrecognised kind, or a column-scoped kind arriving without its
        // column, is not describable here. Silence beats a half-formed line.
        _ => String::new(),
    }
}

/// Bounds a value to one line's worth. A description can be a paragraph, and
/// this line sits inline in a conversation — it reports what was recorded, it
/// is not the place to read it back in full. `contracts show` has the whole of
/// it. Cuts on a character boundary so a multi-byte value cannot panic.
fn elide(value: &str) -> String {
    const MAX: usize = 60;
    if value.chars().count() <= MAX {
        return value.to_string();
    }
    let kept: String = value.chars().take(MAX).collect();
    format!("{}…", kept.trim_end())
}

#[cfg(test)]
#[path = "render_learned_tests.rs"]
mod tests;
