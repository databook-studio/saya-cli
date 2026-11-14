//! One compaction pass: evict the oldest tool-result groups (and any prior
//! digest) into a single digest, keep the newest groups that fit, and fall back
//! to truncating the newest group when it alone exceeds the hard budget.

use super::group_trim::{first_evictable_index, tool_groups, truncate_newest_group};
use super::message_size;

/// One compaction: evict the oldest tool-result groups (and any prior digest),
/// replacing them with a single digest, keeping the most recent groups that
/// still fit under the drain mark. Group and digest discovery is confined to
/// the suffix after the assembly-owned protected prefix. If the newest group
/// alone is above the drain mark it is kept whole while the hard budget holds;
/// only when it exceeds the hard budget too, truncate its largest `tool` result
/// (the one part not yet sent) rather than rewrite a seen message.
pub(super) fn compact(
    messages: &mut Vec<crate::ChatMessage>,
    (budget, drain): (usize, usize),
    protected_prefix_len: usize,
    checkpoint: &super::super::coding_checkpoint::CodingCheckpoint,
) -> (usize, usize) {
    let groups = tool_groups(messages, protected_prefix_len);
    let n = groups.len();
    if n <= 1 {
        // No older group to evict: the newest group alone is over budget (plus
        // any prior digest). Truncate its `tool` result; do not insert a digest
        // for nothing.
        return (
            0,
            truncate_newest_group(messages, budget, protected_prefix_len),
        );
    }
    let Some(region_start) = first_evictable_index(messages, &groups, protected_prefix_len) else {
        return (0, 0);
    };
    // Bytes from each group's start to the end of the conversation, so a
    // candidate keep count is priced in constant time rather than by rescanning
    // the suffix. History follows the configured budget, so the group count is
    // unbounded — the search below must not walk it one group at a time.
    let mut tail_bytes = vec![0usize; n];
    let mut running = 0usize;
    let mut end = messages.len();
    for index in (0..n).rev() {
        let start = groups[index].start;
        running += messages[start..end].iter().map(message_size).sum::<usize>();
        tail_bytes[index] = running;
        end = start;
    }
    let head: usize = messages[..region_start].iter().map(message_size).sum();
    // Keep the most recent groups that fit the drain mark with a single digest
    // replacing the evicted prefix: the largest keep count first, at least one
    // group evicted. When even the newest group alone is above the drain mark,
    // fall back to keeping just it under the hard budget.
    let mut keep = largest_keep_fitting(drain, budget, head, &tail_bytes, checkpoint);
    while keep >= 1 {
        let kept_start = groups[n - keep].start;
        if evict_into_digest(
            messages,
            (region_start, kept_start),
            (drain, budget),
            checkpoint,
        ) {
            return (n - keep, 0);
        }
        keep -= 1;
    }
    if evict_into_digest(
        messages,
        (region_start, groups[n - 1].start),
        (budget, budget),
        checkpoint,
    ) {
        return (n - 1, 0);
    }
    // Evicting all but the newest group still does not fit: evict every group
    // except the newest, insert the digest, then truncate the newest group's
    // largest `tool` result to fit.
    let kept_start = groups[n - 1].start;
    let kept_without_results: usize = messages[kept_start..]
        .iter()
        .filter(|message| message.role != "tool")
        .map(message_size)
        .sum();
    let allowance = budget.saturating_sub(
        head.saturating_add(kept_without_results)
            .saturating_add("assistant".len()),
    );
    let digest = crate::loop_runner::compaction::build_digest_with_checkpoint(
        &messages[region_start..kept_start],
        checkpoint,
        allowance / 2,
    );
    replace_with_digest(messages, region_start, kept_start, digest);
    (
        n - 1,
        truncate_newest_group(messages, budget, protected_prefix_len),
    )
}

/// The largest number of trailing tool groups that can be kept under `target`
/// with one digest replacing the evicted prefix. Keeping fewer groups only
/// frees bytes and grows the digest's allowance, so the fit predicate is
/// monotone and the answer is a binary search rather than a walk — the group
/// count follows the configured budget and is otherwise unbounded. It prices
/// the bytes and the checkpoint's share of the digest; the real eviction
/// confirms the choice, since only it knows the digest header's exact size.
fn largest_keep_fitting(
    target: usize,
    budget: usize,
    head: usize,
    tail_bytes: &[usize],
    checkpoint: &super::super::coding_checkpoint::CodingCheckpoint,
) -> usize {
    let n = tail_bytes.len();
    let fits = |keep: usize| -> bool {
        let kept = tail_bytes[n - keep];
        let allowance =
            target.saturating_sub(head.saturating_add(kept).saturating_add("assistant".len()));
        if allowance < crate::loop_runner::compaction::DIGEST_SENTINEL.len() {
            return false;
        }
        let checkpoint_size =
            crate::loop_runner::compaction::checkpoint_size(checkpoint, budget / 3);
        checkpoint_size == 0 || allowance.saturating_add("assistant".len()) >= checkpoint_size
    };
    if !fits(1) {
        return 0;
    }
    let (mut low, mut high) = (1usize, n - 1);
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if fits(mid) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    low
}

/// Replaces `messages[region_start..kept_start)` with one digest when the head,
/// the digest and the kept groups fit `target`. `budget` is the hard budget the
/// checkpoint's share of the digest is sized against, so the digest keeps its
/// coding facts however low `target` is. Returns whether it replaced anything.
fn evict_into_digest(
    messages: &mut Vec<crate::ChatMessage>,
    (region_start, kept_start): (usize, usize),
    (target, budget): (usize, usize),
    checkpoint: &super::super::coding_checkpoint::CodingCheckpoint,
) -> bool {
    let head: usize = messages[..region_start].iter().map(message_size).sum();
    let kept: usize = messages[kept_start..].iter().map(message_size).sum();
    let allowance =
        target.saturating_sub(head.saturating_add(kept).saturating_add("assistant".len()));
    let checkpoint_size = crate::loop_runner::compaction::checkpoint_size(checkpoint, budget / 3);
    if checkpoint_size > 0 && allowance.saturating_add("assistant".len()) < checkpoint_size {
        return false;
    }
    // A checkpoint needs room for its complete sentinel; otherwise keep
    // evicting groups instead of inserting a misleading partial identity.
    if allowance < crate::loop_runner::compaction::DIGEST_SENTINEL.len() {
        return false;
    }
    let digest = crate::loop_runner::compaction::build_digest_with_checkpoint(
        &messages[region_start..kept_start],
        checkpoint,
        allowance,
    );
    // A digest too small for its own header comes back empty, and one with
    // room only for the header records none of the evicted findings; accepting
    // either would evict history without recording any of it. Refusing keeps
    // the groups for the caller's hard-budget attempt, which has the room.
    if !crate::loop_runner::compaction::records_evicted_findings(
        &digest,
        &messages[region_start..kept_start],
    ) || head + message_size(&digest) + kept > target
    {
        return false;
    }
    replace_with_digest(messages, region_start, kept_start, digest);
    true
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
