//! Tool-result group discovery and the last-resort in-place truncation of the
//! newest group, which is the only part of the conversation not yet sent.

use super::message_size;

/// The index where the evictable region begins: the first tool group, or a
/// prior digest in the unprotected suffix sitting just before it — whichever
/// is earlier. The original system and current-user prefix is never searched.
pub(super) fn first_evictable_index(
    messages: &[crate::ChatMessage],
    groups: &[std::ops::Range<usize>],
    protected_prefix_len: usize,
) -> Option<usize> {
    let first_group = groups.first().map(|range| range.start);
    let first_digest = messages
        .iter()
        .enumerate()
        .skip(protected_prefix_len)
        .find_map(|(index, message)| {
            crate::loop_runner::compaction::is_digest(message).then_some(index)
        });
    match (first_group, first_digest) {
        (Some(g), Some(d)) => Some(g.min(d)),
        (Some(g), None) => Some(g),
        (None, Some(d)) => Some(d),
        (None, None) => None,
    }
}

/// Truncates the newest group's largest `tool` message in place so the whole
/// conversation fits `budget`. The newest group is the one part not yet sent
/// to the provider, so rewriting it does not mislead the model about text it
/// already saw. Stops when the budget is met or no truncation can make
/// progress, never touching any other `tool` message.
pub(super) fn truncate_newest_group(
    messages: &mut [crate::ChatMessage],
    budget: usize,
    protected_prefix_len: usize,
) -> usize {
    let mut truncated = 0;
    loop {
        let total = messages.iter().map(message_size).sum::<usize>();
        if total <= budget {
            return truncated;
        }
        let Some(newest) = tool_groups(messages, protected_prefix_len).last().cloned() else {
            return truncated;
        };
        let Some(idx) = (newest.start + 1..newest.end)
            .filter(|&i| messages[i].role == "tool")
            .max_by_key(|&i| messages[i].content.len())
        else {
            return truncated;
        };
        let content_len = messages[idx].content.len();
        let overhead = total - content_len;
        // The room the result may take: everything but the overhead. A JSON
        // result is rewritten as a valid-JSON envelope of that size, so the
        // model never receives a document cut mid-way.
        let allowed = budget.saturating_sub(overhead);
        if allowed >= content_len {
            return truncated;
        }
        let Some(replacement) = super::trim_envelope::truncate(&messages[idx].content, allowed)
        else {
            return truncated;
        };
        messages[idx].content = replacement;
        truncated += 1;
    }
}

/// Returns the index ranges of complete tool-result groups in `messages`: each
/// is an assistant turn carrying tool calls followed by its consecutive `tool`
/// messages. Ranges are half-open `[start, end)`, suitable for `Vec::drain`.
pub(super) fn tool_groups(
    messages: &[crate::ChatMessage],
    protected_prefix_len: usize,
) -> Vec<std::ops::Range<usize>> {
    let mut groups = Vec::new();
    let mut i = protected_prefix_len;
    while i < messages.len() {
        if !messages[i].tool_calls.is_empty() {
            let start = i;
            i += 1;
            while i < messages.len() && messages[i].role == "tool" {
                i += 1;
            }
            groups.push(start..i);
        } else {
            i += 1;
        }
    }
    groups
}
