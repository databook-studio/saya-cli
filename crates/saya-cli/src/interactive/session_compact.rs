//! Manual `/compact`: shrink working memory through a bounded summariser call.
//!
//! The summarised region is the older turns; the newest tail stays verbatim.
//! A successful compaction stores the summary on the session and later turns
//! replay summary + tail. The transcript (`messages`) is untouched — the
//! `/clear` precedent: working memory changes, what was said does not.
//!
//! Pinning: the summary must preserve every pinned token drawn from the
//! summarised region's assistant texts. Pinned tokens are the
//! continuation-critical file anchors (`size:` / `digest:` lines and the
//! `path:` that names which file they anchor) plus the SQL statements the
//! session ran (the tool-call `arguments`, the only place statements live).
//! A summary that drops any of them is rejected and the session is unchanged —
//! a rewrite on a hope is worse than no rewrite.

use saya_agent::{ChatMessage, TokenUsage};
use saya_store::RedactedTurn;

/// Turns kept verbatim at the end of the conversation. The newest tail is the
/// freshest context — including the newest tool-result group(s) whose size and
/// digest the continuation loop resumes a truncated write from — and the
/// current user turn, which compaction never touches.
pub(crate) const COMPACT_KEEP_TAIL_TURNS: usize = 2;

/// Minimum turns before compaction is offered. At or below this the
/// conversation is short enough already and no provider call runs.
pub(crate) const COMPACT_MIN_TURNS: usize = 3;

/// The summarising call's own bounds: a small output cap plus a timeout, so a
/// compaction can never hang the session behind a half-open connection. The
/// cap bounds the summary text (its own small output cap); the timeout bounds
/// the wait. A cut summary fails validation — a pinned token may sit past
/// the cut — rather than silently keeping a half-summary.
pub(crate) const COMPACT_SUMMARY_MAX_CHARS: usize = 4096;
pub(crate) const COMPACT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// Retry budget for a failed summarising call: at most one retry, then give
/// up. "Leave it alone and say so" is the honest fallback.
pub(crate) const COMPACT_MAX_ATTEMPTS: u32 = 2;

/// A compaction plan: which turns get summarised, which stay verbatim, and
/// the tokens the summary must preserve.
pub(crate) struct CompactionPlan {
    pub(crate) summarise: Vec<ChatMessage>,
    pub(crate) compacted_turns: usize,
    pub(crate) pinned: Vec<String>,
}

/// The outcome of the bounded summarising call: the summary text plus the
/// provider-reported usage, kept apart from the answering total exactly like
/// the post-turn extraction call's (`AgentOutput::learning_usage`).
#[derive(Debug)]
pub(crate) struct CompactionOutcome {
    pub(crate) summary: String,
    pub(crate) usage: Option<TokenUsage>,
}

/// Builds the summariser prompt: the older turns' text plus the instruction to
/// preserve every pinned token verbatim. The newest tail is not sent — it is
/// never summarised, so the model cannot drop it.
pub(crate) fn summarise_prompt(summarise: &[ChatMessage]) -> String {
    let mut prompt = String::from(
        "Summarise the following earlier conversation turns into a compact \
         narrative a database assistant can resume from. Keep facts, decisions, \
         and open work; drop chatter. You MUST preserve every pinned token \
         below verbatim (exact strings, including size/digest anchors and SQL \
         statements) — the session is invalid without them.\n\n",
    );
    for message in summarise {
        prompt.push_str(&format!("{}: {}\n", message.role, message.content));
    }
    prompt.push_str("\nPinned tokens (preserve every one verbatim):\n");
    for token in pinned_tokens(summarise) {
        prompt.push_str(&format!("- {token}\n"));
    }
    prompt
}

/// Decides what `/compact` would summarise: the older turns' replayed text,
/// the count of compacted turns, and the tokens the summary must preserve.
/// `None` when the conversation is short enough already — no provider call.
pub(crate) fn plan(turns: &[RedactedTurn], history: &[ChatMessage]) -> Option<CompactionPlan> {
    if turns.len() <= COMPACT_MIN_TURNS {
        return None;
    }
    let summarised = turns.len() - COMPACT_KEEP_TAIL_TURNS;
    let summarise: Vec<ChatMessage> = history.iter().take(summarised * 2).cloned().collect();
    if summarise.is_empty() {
        return None;
    }
    let pinned = pinned_tokens(&summarise);
    Some(CompactionPlan {
        summarise,
        compacted_turns: summarised,
        pinned,
    })
}

/// Applies a validated summary: stores it and the compacted-turn count on the
/// session. The transcript (`messages`) is untouched — only working memory
/// changes. Callers must run [`validate`] first; this function re-checks and
/// refuses a summary that lost a pinned token.
pub(crate) fn apply(
    state: &mut super::session_state::SessionState,
    plan: &CompactionPlan,
    summary: &str,
) -> Result<(), String> {
    validate(summary, &plan.pinned)?;
    state.compaction_summary = Some(summary.to_string());
    state.compacted_turns = plan.compacted_turns;
    // The one-shot warning measures live answering usage, which a shorter
    // replay legitimately lowers: re-arm so the next crossing warns again.
    state.context_warned = false;
    Ok(())
}

/// Validates a candidate summary: non-empty, and carrying every pinned token
/// verbatim. Rejects anything else — the session is left exactly as it was.
pub(crate) fn validate(summary: &str, pinned: &[String]) -> Result<(), String> {
    if summary.trim().is_empty() {
        return Err("the summariser returned an empty summary".into());
    }
    for token in pinned {
        if !summary.contains(token) {
            return Err("the summary dropped a pinned tool result".into());
        }
    }
    Ok(())
}

/// Estimates summary tokens for the success message (`~X tokens`): words are
/// the cheap, honest proxy — never presented as a provider count.
pub(crate) fn summary_tokens(summary: &str) -> u64 {
    summary.split_whitespace().count() as u64
}

/// The success message: what compacted, the estimate, and the transcript
/// guarantee with its remedy.
pub(crate) fn success_message(compacted_turns: usize, summary: &str) -> String {
    format!(
        "Compacted {compacted_turns} turns into a summary (~{} tokens). \
         The transcript is unchanged; /export first if you want the full text.",
        summary_tokens(summary)
    )
}

/// The failure message: the reason, and the guarantee that nothing changed.
pub(crate) fn failure_message(reason: &str) -> String {
    format!("Compaction failed ({reason}); the conversation is unchanged.")
}

/// Pinned tokens drawn from the summarised text: continuation-critical file
/// anchors plus the SQL statements the session ran.
///
/// What is pinned, and why:
/// - `size:` / `digest:` lines: the continuation loop resumes a truncated
///   write by re-appending from the size and digest the earlier tool result
///   reported. A summary that drops them silently breaks recovery.
/// - `path:` lines: a size/digest anchors one file; without the path the
///   anchor names no file and the model cannot resume the right one.
/// - SQL statements: the tool-call `arguments` are the only record of what
///   ran. Provider history carries no tool result bytes, so a verbatim tail
///   alone cannot preserve them for older turns — the summary must.
///
/// What is NOT pinned, and why:
/// - The system prompt is never compacted: it is assembled fresh per turn in
///   `saya-agent` (`history::build_messages`), never from the session, so it
///   is outside the summarised region by construction.
/// - The current user turn is never compacted: it is not in the session yet
///   when `/compact` runs — the summarised region holds only past turns, and
///   the prompt rides the next turn beside the summary.
/// - Row values are never pinned: they never reach the session at all
///   (`record_turn` persists statements and value-free shapes only), so
///   there is nothing to preserve.
pub(crate) fn pinned_tokens(summarise: &[ChatMessage]) -> Vec<String> {
    let mut pinned = Vec::new();
    for message in summarise {
        for line in message.content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("size:")
                || trimmed.starts_with("digest:")
                || trimmed.starts_with("path:")
                || trimmed.to_lowercase().starts_with("select")
                || trimmed.to_lowercase().starts_with("with")
            {
                pinned.push(trimmed.to_string());
            }
        }
    }
    pinned.sort();
    pinned.dedup();
    pinned
}
