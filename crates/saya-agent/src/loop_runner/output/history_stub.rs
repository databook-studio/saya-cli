//! Per-request history policy: a long history is bounded only when it must be.
//!
//! A provider request does not need to re-send every old tool result forever,
//! but replacing one the model still needs is worse than sending it: the model
//! reads a file to edit it, and a stub that says "call the tool again" turns
//! one read into a loop. So the policy is keyed to a budget, not to age.
//!
//! The history is replayed whole until the tool-result bytes in it exceed
//! [`budget_bytes`]: 40 % of a known model window at four bytes per token,
//! bounded by [`HISTORY_BUDGET_CEILING_BYTES`], or that fixed ceiling alone
//! when the window is unknown. A typical short job never reaches it and sends
//! exactly the bytes it sent before this policy existed.
//!
//! Over budget, a pass replaces results in three stages, oldest first:
//!
//! 1. **Superseded** results that can never be needed again: a read whose path
//!    a later read covers at the same or a wider range, or that a later
//!    successful write or edit changed; and a shell, search or database result
//!    whose exact call ran again later. Their stub names the later call and
//!    never tells the model to call again.
//! 2. **Old non-read output** outside the last [`LAST_WHOLE_GROUPS`] groups.
//! 3. **Reads** only while the history is still above the hard ceiling (80 %
//!    of a known window, or the fixed ceiling when it is unknown). Their stub
//!    says the file must be re-read before editing.
//!
//! A pass drains to the low-water mark, half the budget, in one batch, and
//! records where the settled region ends in [`HistoryStubState`]. Later
//! requests append without rewriting that region until the budget is exceeded
//! again, so rewrites are rare batched events rather than one per request. A
//! stub is one-way and idempotent: once written it is never rewritten.
//!
//! Arguments and excerpt lines pass through [`saya_types::redact`], and the
//! whole stub is capped, so a secret in a tool result is never re-sent by this
//! lane. A result no larger than a stub is kept whole: replacing it would make
//! the request bigger.

#[path = "history_stub/budget.rs"]
mod budget;
#[path = "history_stub/calls.rs"]
mod calls;
#[path = "history_stub/render.rs"]
mod render;

pub(in crate::loop_runner) use budget::{HistoryStubState, budget_bytes, hard_ceiling_bytes};

use crate::ChatMessage;
use std::ops::Range;

/// Leading sentinel on a stubbed tool result, so a later request recognises it
/// and leaves it alone.
pub(super) const STUB_SENTINEL: &str = "…[history-stub:";

/// Hard cap on one stub. A stub is a pointer back to the result, not a
/// summary of it; keeping it small is the point of the policy.
pub(in crate::loop_runner) const MAX_STUB_BYTES: usize = 900;

/// Ceiling on the tool-result history when the model window is unknown, and
/// the upper bound when it is known.
pub(in crate::loop_runner) const HISTORY_BUDGET_CEILING_BYTES: usize = 256 * 1024;

/// The newest groups whose results always stay whole. The newest group is the
/// turn just executed; the rest give the model recent context to compare.
const LAST_WHOLE_GROUPS: usize = 6;

/// Applies the policy for a resolved model window.
pub(in crate::loop_runner) fn stub_old_tool_results(
    messages: &mut [ChatMessage],
    protected_prefix_len: usize,
    window_tokens: Option<u64>,
    state: &mut HistoryStubState,
) -> usize {
    let budget = budget_bytes(window_tokens);
    let hard_ceiling = hard_ceiling_bytes(window_tokens);
    stub_with_limits(messages, protected_prefix_len, budget, hard_ceiling, state)
}

/// Applies the policy with explicit thresholds. The window-derived wrapper
/// above supplies production values; tests pin the thresholds directly.
pub(in crate::loop_runner) fn stub_with_limits(
    messages: &mut [ChatMessage],
    protected_prefix_len: usize,
    budget: usize,
    hard_ceiling: usize,
    state: &mut HistoryStubState,
) -> usize {
    // The budget check is one walk and no allocation: below it, a request does
    // exactly the work it did before this policy existed.
    let mut total = tool_result_bytes(messages, protected_prefix_len);
    if total <= budget {
        return 0;
    }
    let start = settled_start(messages, protected_prefix_len, state);
    let groups = super::group_trim::tool_groups(messages, start);
    #[cfg(test)]
    work::group_list_built();
    if groups.is_empty() {
        return 0;
    }
    let calls = calls::collect(messages, &groups);
    let low_water = budget / 2;
    let last_whole = groups.len().saturating_sub(LAST_WHOLE_GROUPS);
    let mut stubbed = 0;
    let mut last_stubbed = None;

    // 1. Superseded results can never be needed again, wherever they sit.
    for (position, call) in calls.iter().enumerate() {
        if total <= low_water {
            break;
        }
        let Some(index) = call.result_index else {
            continue;
        };
        if !needs_stub(&messages[index]) {
            continue;
        }
        let Some(later) = calls::superseded_by(&calls, position) else {
            continue;
        };
        stubbed += rewrite(
            messages,
            index,
            call,
            render::Kind::Superseded(later),
            &mut total,
        );
        last_stubbed = Some(index);
    }

    // 2. Old non-read output outside the newest groups.
    for call in &calls {
        if total <= low_water {
            break;
        }
        if call.group >= last_whole || !call.is_old_output() {
            continue;
        }
        let Some(index) = call.result_index else {
            continue;
        };
        if !needs_stub(&messages[index]) {
            continue;
        }
        stubbed += rewrite(messages, index, call, render::Kind::Old, &mut total);
        last_stubbed = Some(index);
    }

    // 3. Reads only above the hard ceiling: the last resort.
    if total > hard_ceiling {
        for call in &calls {
            if total <= low_water {
                break;
            }
            if !call.is_read() {
                continue;
            }
            let Some(index) = call.result_index else {
                continue;
            };
            if !needs_stub(&messages[index]) {
                continue;
            }
            stubbed += rewrite(messages, index, call, render::Kind::WorkingCopy, &mut total);
            last_stubbed = Some(index);
        }
    }

    if stubbed > 0
        && let Some(index) = last_stubbed
    {
        state.settled_through = settled_marker(messages, &groups, index);
    }
    stubbed
}

/// Tool-result bytes in the history the policy may consider.
fn tool_result_bytes(messages: &[ChatMessage], from: usize) -> usize {
    messages[from..]
        .iter()
        .filter(|message| message.role == "tool")
        .fold(0usize, |total, message| {
            total.saturating_add(message.content.len())
        })
}

/// Where a pass may start: just after the last message it settled. If that
/// message is gone (a compaction removed it), the whole history is considered
/// again — a compaction has already broken the cached prefix.
fn settled_start(
    messages: &[ChatMessage],
    protected_prefix_len: usize,
    state: &HistoryStubState,
) -> usize {
    state
        .settled_through
        .as_deref()
        .and_then(|id| {
            messages.iter().position(|message| {
                message.role == "tool" && message.tool_call_id.as_deref() == Some(id)
            })
        })
        .map_or(protected_prefix_len, |index| index + 1)
}

/// The id a later pass uses to find the end of this pass's settled region: the
/// last tool message of the group that was rewritten last.
fn settled_marker(
    messages: &[ChatMessage],
    groups: &[Range<usize>],
    index: usize,
) -> Option<String> {
    let range = groups.iter().find(|range| range.contains(&index))?;
    (range.start + 1..range.end)
        .rev()
        .find(|&i| messages[i].role == "tool")
        .and_then(|i| messages[i].tool_call_id.clone())
}

/// Whether this message is a tool result the pass would rewrite: large enough
/// to be worth a stub and not already stubbed.
fn needs_stub(message: &ChatMessage) -> bool {
    message.role == "tool"
        && message.content.len() > MAX_STUB_BYTES
        && !message.content.starts_with(STUB_SENTINEL)
}

/// Replaces one result and returns 1. `total` is kept in step so the pass can
/// stop at the low-water mark without re-measuring the history.
fn rewrite(
    messages: &mut [ChatMessage],
    index: usize,
    call: &calls::Call,
    kind: render::Kind<'_>,
    total: &mut usize,
) -> usize {
    let replacement = render::stub(kind, call, &messages[index].content);
    *total = total.saturating_sub(
        messages[index]
            .content
            .len()
            .saturating_sub(replacement.len()),
    );
    messages[index].content = replacement;
    1
}

/// Test-only tallies of the per-request work a pass does, so a test can assert
/// that a pass with nothing to stub allocates nothing. Thread-local: a test
/// reads only its own thread's counts, so the suite can run in parallel.
#[cfg(test)]
pub(super) mod work {
    use std::cell::Cell;

    thread_local! {
        static GROUP_LISTS: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn group_list_built() {
        GROUP_LISTS.with(|count| count.set(count.get() + 1));
    }

    pub(crate) fn reset() {
        GROUP_LISTS.with(|count| count.set(0));
    }

    pub(crate) fn taken() -> usize {
        GROUP_LISTS.with(Cell::get)
    }
}
