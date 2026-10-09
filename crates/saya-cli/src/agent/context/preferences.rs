//! Projecting a worktree's explicit preferences into one bounded, untrusted,
//! labelled context block on the request's user turn.
//!
//! Preferences are user-confirmed guidance (decision 18): they ride the same
//! untrusted lane as the instruction sources — never the system prompt — but
//! they are independent of the `[memory]` settings and the database privacy
//! gate: a state database, a bound worktree, and at least one preference is
//! enough. Oldest first, whole items only, capped at
//! [`MAX_PREFERENCE_REQUEST_BYTES`] and the request's remaining budget; any
//! store failure yields no block and the turn proceeds.

use std::path::Path;

use saya_agent::{ContextBlock, turn_bytes};
use saya_store::{ProjectPreferenceStore, SqliteStateStore};
use saya_types::ProjectPreference;

use super::sources::{HISTORY_RESERVE_FRACTION, MIN_BLOCK_BODY_BYTES, UNTRUSTED_PREFIX};

/// The user-turn label for the preferences block.
pub(crate) const PREFERENCE_BLOCK_LABEL: &str = "project-preferences";

/// Request bytes the preferences block may claim: the aggregate recall cap.
pub(crate) const MAX_PREFERENCE_REQUEST_BYTES: usize = 16 * 1024;

/// The fixed header line above the preference items.
const HEADER: &str =
    "Explicit project preferences confirmed by the user for this worktree (oldest first):";

/// One preference as one list line: redacted again first — defensive; the
/// store screened the text at add time — then newlines collapsed so an item
/// stays one line. Ids and timestamps never ride.
fn preference_line(item: &ProjectPreference) -> String {
    let redacted = saya_types::redact(&item.text);
    format!("- {}", redacted.replace('\n', " ⏎ "))
}

/// The block body: framing line, blank line, header, one item per line.
fn body(lines: &[String]) -> String {
    let mut body = format!("{UNTRUSTED_PREFIX}\n\n{HEADER}");
    for line in lines {
        body.push('\n');
        body.push_str(line);
    }
    body
}

/// The exact post-escape, post-wrapper request bytes one block adds.
fn cost(block: &ContextBlock) -> usize {
    turn_bytes(None, std::slice::from_ref(block), "").saturating_sub(turn_bytes(None, &[], ""))
}

/// The budget the preferences block may claim: what the request still has
/// after the system prompt, the question, and every block already pushed —
/// including the spliced instruction blocks — capped by the preference
/// ceiling and by the same history-reserve fraction the instructions honour.
pub(crate) fn remaining_request_bytes(used_so_far: usize, budget: usize) -> usize {
    budget
        .saturating_sub(used_so_far)
        .min(MAX_PREFERENCE_REQUEST_BYTES)
        .min(
            (budget * HISTORY_RESERVE_FRACTION.0 / HISTORY_RESERVE_FRACTION.1)
                .saturating_sub(used_so_far),
        )
}

/// One turn's preference projection: at most one block, plus what it held.
/// The default is the empty projection — what every refusal yields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PreferenceProjection {
    pub(crate) blocks: Vec<ContextBlock>,
    pub(crate) included: usize,
    pub(crate) omitted: usize,
}

/// Renders `items` oldest-first into at most one block, whole items only:
/// the first that does not fit stops the list, and the block ends with a
/// visible count of what was left out. A budget below the block floor — or
/// too small for the header and the first item — drops the block entirely,
/// and the omission note itself must fit the budget it claims.
pub(crate) fn render_preferences(
    items: Vec<ProjectPreference>,
    remaining: usize,
) -> PreferenceProjection {
    let mut items = items;
    items.sort_by(|a, b| {
        (a.created_unix_ms, a.id.as_str()).cmp(&(b.created_unix_ms, b.id.as_str()))
    });
    let lines: Vec<String> = items.iter().map(preference_line).collect();
    let remaining = remaining.min(MAX_PREFERENCE_REQUEST_BYTES);
    if remaining < MIN_BLOCK_BODY_BYTES {
        return PreferenceProjection::default();
    }
    let mut best = PreferenceProjection::default();
    for keep in 1..=lines.len() {
        let mut body = body(&lines[..keep]);
        let omitted = lines.len() - keep;
        let truncated = omitted > 0;
        if truncated {
            body.push_str(&format!(
                "\n… {omitted} more preference(s) not included (budget)"
            ));
        }
        let block = ContextBlock {
            label: PREFERENCE_BLOCK_LABEL.to_string(),
            body,
            truncated,
        };
        if cost(&block) > remaining {
            break;
        }
        best = PreferenceProjection {
            blocks: vec![block],
            included: keep,
            omitted,
        };
    }
    debug_assert!(
        best.blocks.is_empty() || best.included + best.omitted == items.len(),
        "the block accounts for every preference, held or omitted"
    );
    best
}

/// Reads the bound worktree's preferences and projects at most one block.
/// Never errors: no state database, no bound root, an unusable root, or any
/// store failure all yield an empty projection and the turn proceeds.
pub(crate) async fn project_preferences(
    state_db: Option<&SqliteStateStore>,
    root: Option<&Path>,
    remaining: usize,
) -> PreferenceProjection {
    let Some(state_db) = state_db else {
        return PreferenceProjection::default();
    };
    let Some(root) = root else {
        return PreferenceProjection::default();
    };
    let Ok(worktree) = crate::commands::preferences::worktree_id_for(root) else {
        return PreferenceProjection::default();
    };
    match state_db.list_project_preferences(&worktree).await {
        Ok(items) => render_preferences(items, remaining),
        Err(_) => PreferenceProjection::default(),
    }
}

#[cfg(test)]
#[path = "preferences_tests.rs"]
mod preference_context_tests;
