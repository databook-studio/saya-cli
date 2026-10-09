/// Approximate serialized size of a message, including tool-call arguments,
/// which dominate real growth during multi-turn runs.
pub(in crate::loop_runner) fn message_size(message: &crate::ChatMessage) -> usize {
    message.content.len()
        + message.role.len()
        + message
            .tool_calls
            .iter()
            .map(|call| {
                call.id.len()
                    + call.name.len()
                    + serde_json::to_string(&call.arguments).map_or(0, |text| text.len())
            })
            .sum::<usize>()
}

/// A completion summary for a tool result, flagged when the result was
/// truncated to fit the conversation budget so the model is told it saw a cut
/// result rather than a complete one — the model must not be misled.
pub(in crate::loop_runner) fn completion_summary(base: &str, truncated: bool) -> String {
    if truncated {
        format!("{base} (truncated)")
    } else {
        base.to_owned()
    }
}

/// The marker appended to an in-place-truncated tool message so the model
/// knows the result it received was cut.
pub(super) const TRUNCATED_MARKER: &str =
    "…[truncated: tool result exceeded the conversation byte budget]";

/// Enforces the intra-loop conversation byte budget in place, never aborting
/// the run. Under the budget the conversation is left untouched. Over it, a
/// single compaction evicts the oldest tool-result groups and replaces them
/// with one short digest (the statements that ran and the shapes of their
/// results — never row values), keeping the system message and the most
/// recent groups intact. One compaction rewrites the prefix once, so a prompt
/// cache misses once per compaction rather than once per turn; the loop then
/// appends again until the budget is exceeded anew.
///
/// An already-sent tool message is never rewritten: only the newest group —
/// the one part of the conversation not yet sent to the provider at the point
/// of trimming — may be truncated, and only when it alone (with the digest)
/// still exceeds the budget.
pub(in crate::loop_runner) fn trim_to_budget(
    messages: &mut Vec<crate::ChatMessage>,
    budget: usize,
    protected_prefix_len: usize,
) {
    let total = messages.iter().map(super::message_size).sum::<usize>();
    if total <= budget {
        return;
    }
    compact(messages, budget, protected_prefix_len);
}

/// One compaction: evict the oldest tool-result groups (and any prior digest),
/// replacing them with a single digest, keeping the most recent groups that
/// still fit. Group and digest discovery is confined to the suffix after the
/// assembly-owned protected prefix. If keeping only the newest group with the
/// digest still does not fit, truncate its largest `tool` result (the one part
/// not yet sent) rather than rewrite a seen message.
fn compact(messages: &mut Vec<crate::ChatMessage>, budget: usize, protected_prefix_len: usize) {
    let groups = tool_groups(messages, protected_prefix_len);
    let n = groups.len();
    if n <= 1 {
        // No older group to evict: the newest group alone is over budget (plus
        // any prior digest). Truncate its `tool` result; do not insert a digest
        // for nothing.
        truncate_newest_group(messages, budget, protected_prefix_len);
        return;
    }
    let Some(region_start) = first_evictable_index(messages, &groups, protected_prefix_len) else {
        return;
    };
    // Keep the most recent groups that fit with a single digest replacing the
    // evicted prefix. Try the largest keep count first; evict at least one.
    for keep in (1..n).rev() {
        let kept_start = groups[n - keep].start;
        let digest =
            crate::loop_runner::compaction::build_digest(&messages[region_start..kept_start]);
        let head: usize = messages[..region_start]
            .iter()
            .map(super::message_size)
            .sum();
        let kept: usize = messages[kept_start..].iter().map(super::message_size).sum();
        if head + super::message_size(&digest) + kept <= budget {
            replace_with_digest(messages, region_start, kept_start, digest);
            return;
        }
    }
    // Evicting all but the newest group still does not fit: evict every group
    // except the newest, insert the digest, then truncate the newest group's
    // largest `tool` result to fit.
    let kept_start = groups[n - 1].start;
    let digest = crate::loop_runner::compaction::build_digest(&messages[region_start..kept_start]);
    replace_with_digest(messages, region_start, kept_start, digest);
    truncate_newest_group(messages, budget, protected_prefix_len);
}

/// Drains `messages[start..end)` and inserts `digest` at `start` in its place.
fn replace_with_digest(
    messages: &mut Vec<crate::ChatMessage>,
    start: usize,
    end: usize,
    digest: crate::ChatMessage,
) {
    messages.drain(start..end);
    messages.insert(start, digest);
}

/// The index where the evictable region begins: the first tool group, or a
/// prior digest in the unprotected suffix sitting just before it — whichever
/// is earlier. The original system and current-user prefix is never searched.
fn first_evictable_index(
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
fn truncate_newest_group(
    messages: &mut [crate::ChatMessage],
    budget: usize,
    protected_prefix_len: usize,
) {
    loop {
        let total = messages.iter().map(super::message_size).sum::<usize>();
        if total <= budget {
            return;
        }
        let Some(newest) = tool_groups(messages, protected_prefix_len).last().cloned() else {
            return;
        };
        let Some(idx) = (newest.start + 1..newest.end)
            .filter(|&i| messages[i].role == "tool")
            .max_by_key(|&i| messages[i].content.len())
        else {
            return;
        };
        let content_len = messages[idx].content.len();
        let overhead = total - content_len;
        let target = budget.saturating_sub(overhead + TRUNCATED_MARKER.len());
        if target >= content_len {
            return;
        }
        let bound = crate::loop_runner::tools::floor_boundary(&messages[idx].content, target);
        if bound + TRUNCATED_MARKER.len() >= content_len {
            return;
        }
        messages[idx].content.truncate(bound);
        messages[idx].content.push_str(TRUNCATED_MARKER);
    }
}

/// Returns the index ranges of complete tool-result groups in `messages`: each
/// is an assistant turn carrying tool calls followed by its consecutive `tool`
/// messages. Ranges are half-open `[start, end)`, suitable for `Vec::drain`.
fn tool_groups(
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
