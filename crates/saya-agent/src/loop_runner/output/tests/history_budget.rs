//! The working suffix follows the configured context budget.
//!
//! There are no fixed hard caps: history grows until the conversation would
//! exceed 90% of the configured budget; one compaction then drains it to 60%,
//! keeping the protected prefix (the prompt-cache prefix) byte-identical.

use super::super::*;
use super::compaction::group;
use crate::ChatMessage;
use crate::loop_runner::compaction::is_digest;
use std::sync::Mutex;

const MIB: usize = 1024 * 1024;
const PROTECTED: usize = 2;

#[derive(Default)]
struct CompactionLog(Mutex<Vec<crate::ContextCompaction>>);

impl CompactionLog {
    fn all(&self) -> Vec<crate::ContextCompaction> {
        self.0.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl crate::AgentEventSink for CompactionLog {
    fn observe_compaction(&self, observation: crate::ContextCompaction) {
        self.0.lock().unwrap().push(observation);
    }
    async fn emit(&self, _: crate::AgentEvent) {}
}

fn size(messages: &[ChatMessage]) -> usize {
    messages.iter().map(message_size).sum()
}

fn groups(messages: &[ChatMessage]) -> usize {
    messages.iter().filter(|m| !m.tool_calls.is_empty()).count()
}

fn result(bytes: usize) -> serde_json::Value {
    serde_json::json!({"row_count":1,"columns":["one"],"rows":[["x".repeat(bytes)]]})
}

/// The system + user prefix followed by `count` tool groups, each with a
/// `result_bytes`-byte result.
fn history(count: usize, result_bytes: usize) -> Vec<ChatMessage> {
    let mut messages = vec![
        ChatMessage::text("system", "sys"),
        ChatMessage::text("user", "ask"),
    ];
    for index in 0..count {
        messages.extend(group(
            &format!("call-{index:04}"),
            "SELECT 1",
            result(result_bytes),
        ));
    }
    messages
}

/// (a) The suffix uses the configured budget, not a 128 KiB working cap.
/// A 4 MiB budget holds 3 MiB of tool history without compacting at all.
#[test]
fn three_mib_of_tool_history_under_a_four_mib_budget_is_not_compacted() {
    let budget = 4 * MIB;
    let mut messages = history(48, 64 * 1024);
    let suffix = size(&messages[PROTECTED..]);
    assert!(
        suffix >= 3 * MIB,
        "the test must plant at least 3 MiB of tool history: {suffix}"
    );
    let snapshot = messages.clone();
    let log = CompactionLog::default();
    trim_to_budget(
        &mut messages,
        ContextBudget::new(budget, None),
        PROTECTED,
        Some(&log),
        &Default::default(),
    );
    assert!(
        log.all().is_empty(),
        "3 MiB under a 4 MiB budget is well under the trigger: {:?}",
        log.all()
    );
    assert_eq!(
        messages, snapshot,
        "an under-trigger conversation is untouched"
    );
}

/// (b) Many groups are not a compaction trigger of their own. Sixty small
/// groups under 90% of the budget leave the conversation untouched; there is
/// no 48-group cap.
#[test]
fn sixty_small_groups_under_the_trigger_are_not_compacted() {
    let budget = 4 * MIB;
    let mut messages = history(60, 64);
    let total = size(&messages);
    assert!(
        groups(&messages) == 60 && total < budget / 100 * 90,
        "the test must plant 60 groups under 90% of the budget: {total}"
    );
    let snapshot = messages.clone();
    let log = CompactionLog::default();
    trim_to_budget(
        &mut messages,
        ContextBudget::new(budget, None),
        PROTECTED,
        Some(&log),
        &Default::default(),
    );
    assert!(
        log.all().is_empty(),
        "60 groups under the byte trigger must not compact: {:?}",
        log.all()
    );
    assert_eq!(
        messages, snapshot,
        "an under-trigger conversation is untouched"
    );
}

/// (c) Crossing 90% of the budget starts exactly one compaction, and that
/// compaction drains to 60% — keeping the most recent groups that fit, not
/// half of a separate suffix cap.
#[test]
fn crossing_the_trigger_compacts_once_and_drains_to_sixty_percent() {
    let budget = MIB;
    let mut messages = history(10, 100 * 1024);
    let before = size(&messages);
    assert!(
        before > budget / 100 * 90 && before < budget,
        "the test must cross the 90% trigger without exceeding the budget: {before} vs {budget}"
    );
    let log = CompactionLog::default();
    trim_to_budget(
        &mut messages,
        ContextBudget::new(budget, None),
        PROTECTED,
        Some(&log),
        &Default::default(),
    );
    let observed = log.all();
    assert_eq!(observed.len(), 1, "one trigger reports one compaction");
    let after = size(&messages);
    let drain = budget / 100 * 60;
    assert!(
        after <= drain,
        "a compaction must drain to 60% of the budget: {after} vs {drain}"
    );
    let one_group = size(&group("call-next", "SELECT 1", result(100 * 1024)));
    assert!(
        after + one_group > drain,
        "the drain must keep every group that still fits under 60%: \
         {after} + {one_group} vs {drain}"
    );
    let evicted = observed[0].evicted_tool_groups as usize;
    assert_eq!(
        evicted,
        10 - groups(&messages),
        "the observation must report the groups actually evicted"
    );
    assert_eq!(
        messages.iter().filter(|m| is_digest(m)).count(),
        1,
        "exactly one digest replaces the evicted groups"
    );
}

/// (d) The protected prefix — the bytes a provider prefix cache keys on —
/// is byte-identical across compactions. Two successive compactions leave it
/// untouched, and each observation reports it unchanged.
#[test]
fn the_protected_prefix_is_byte_identical_across_compactions() {
    let budget = MIB;
    let mut messages = history(10, 100 * 1024);
    let prefix = messages[..PROTECTED].to_vec();
    let prefix_bytes = size(&prefix);
    let log = CompactionLog::default();
    let mut next = 0;
    for round in 0..2 {
        // Append until the trigger is crossed again, as the loop does turn by
        // turn, so each round genuinely compacts.
        while size(&messages) <= budget / 100 * 90 {
            messages.extend(group(
                &format!("call-round-{round}-{next}"),
                "SELECT 1",
                result(100 * 1024),
            ));
            next += 1;
        }
        let seen = log.all().len();
        trim_to_budget(
            &mut messages,
            ContextBudget::new(budget, None),
            PROTECTED,
            Some(&log),
            &Default::default(),
        );
        assert_eq!(
            messages[..PROTECTED],
            prefix[..],
            "round {round}: the prefix must not be rewritten"
        );
        assert_eq!(
            size(&messages[..PROTECTED]),
            prefix_bytes,
            "round {round}: the prefix bytes must be unchanged"
        );
        let observed = log.all();
        assert_eq!(
            observed.len(),
            seen + 1,
            "round {round}: the round must compact once"
        );
        assert!(observed[seen].protected_prefix_unchanged);
        assert_eq!(observed[seen].protected_prefix_messages, PROTECTED as u64);
    }
}
