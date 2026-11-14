//! A compaction drains the working history to 60% of the configured budget, so
//! the turns after it append to an unchanged prefix instead of compacting again.

use super::super::*;
use super::compaction::group;
use crate::ChatMessage;
use crate::loop_runner::coding_checkpoint::{CodingCheckpoint, observe};
use crate::loop_runner::compaction::is_digest;
use std::sync::Mutex;

const PROTECTED: usize = 2;
const LARGE_BUDGET: usize = 4 * 1024 * 1024;

/// `budget` scaled by `percent`, exactly as the budget trim scales it.
fn share(budget: usize, percent: usize) -> usize {
    budget.saturating_mul(percent) / 100
}

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

fn digests(messages: &[ChatMessage]) -> usize {
    messages.iter().filter(|m| is_digest(m)).count()
}

fn conversation() -> Vec<ChatMessage> {
    vec![
        ChatMessage::text("system", "sys"),
        ChatMessage::text("user", "ask"),
    ]
}

fn result(bytes: usize) -> serde_json::Value {
    serde_json::json!({"row_count":1,"columns":["one"],"rows":[["x".repeat(bytes)]]})
}

struct Compaction {
    evicted: u64,
    groups: usize,
    bytes: usize,
    drain: usize,
}

struct Run {
    compactions: Vec<Compaction>,
    group_bytes: usize,
}

impl Run {
    fn evicted(&self) -> Vec<u64> {
        self.compactions.iter().map(|c| c.evicted).collect()
    }
}

/// Appends `turns` tool groups and trims after each exactly as the loop does,
/// checking on every turn what must hold whether or not it compacts.
fn drive(budget: usize, turns: usize, result_bytes: usize) -> Run {
    let mut checkpoint = CodingCheckpoint::default();
    checkpoint.observe_serial(&observe(
        "follow_up_child",
        &serde_json::json!({}),
        Ok(&serde_json::json!({
            "agent_id":"child-001",
            "previous_invocation_id":"invocation-001-before",
            "invocation_id":"invocation-001-after",
            "previous_completion_sequence":1,
            "checkout_id":"checkout-001"
        })),
    ));
    let log = CompactionLog::default();
    let mut messages = conversation();
    let mut compactions = Vec::new();
    let mut group_bytes = 0;
    for turn in 0..turns {
        let previous = messages.clone();
        let newest = group(&format!("call-{turn:03}"), "SELECT 1", result(result_bytes));
        group_bytes = size(&newest);
        messages.extend(newest.clone());
        let (before_bytes, before_groups) = (size(&messages), groups(&messages));
        let seen = log.all().len();
        trim_to_budget(
            &mut messages,
            ContextBudget::new(budget, None),
            PROTECTED,
            Some(&log),
            &checkpoint,
        );
        assert_eq!(
            messages[..PROTECTED],
            previous[..PROTECTED],
            "turn {turn}: the protected prefix was rewritten"
        );
        let observed = log.all();
        if observed.len() == seen {
            assert_eq!(
                messages[..previous.len()],
                previous[..],
                "turn {turn}: the prefix changed without a compaction"
            );
            assert_eq!(messages.len(), previous.len() + newest.len());
            continue;
        }
        let compaction = observed[seen];
        assert_eq!(compaction.before_bytes as usize, before_bytes);
        assert_eq!(compaction.after_bytes as usize, size(&messages));
        assert_eq!(
            compaction.evicted_tool_groups as usize,
            before_groups - groups(&messages),
            "turn {turn}: the observation must report the groups actually evicted"
        );
        assert_eq!(compaction.truncated_tool_messages, 0);
        assert!(compaction.protected_prefix_unchanged);
        assert_eq!(compaction.protected_prefix_messages, PROTECTED as u64);
        assert_eq!(digests(&messages), 1, "turn {turn}: exactly one digest");
        assert_eq!(messages[messages.len() - newest.len()..], newest[..]);
        let digest = messages.iter().find(|m| is_digest(m)).unwrap();
        assert!(
            digest.content.contains("\"agent_id\":\"child-001\""),
            "turn {turn}: the digest must carry the coding checkpoint facts"
        );
        compactions.push(Compaction {
            evicted: compaction.evicted_tool_groups,
            groups: groups(&messages),
            bytes: size(&messages),
            drain: share(budget, 60),
        });
    }
    Run {
        compactions,
        group_bytes,
    }
}

/// Past the trigger a compaction drains to 60% of the configured budget, so the
/// next one needs at least the 30% between the drain and the trigger in new
/// history — one compaction per trigger, never a per-turn trickle.
#[test]
fn a_compaction_drains_to_sixty_percent_so_the_next_needs_new_history() {
    let turns = 120;
    for (budget, result_bytes) in [(LARGE_BUDGET, 64 * 1024), (64 * 1024, 2 * 1024)] {
        let run = drive(budget, turns, result_bytes);
        assert!(
            !run.compactions.is_empty(),
            "budget {budget}: no compaction"
        );
        let gap = (share(budget, 90) - share(budget, 60)) / run.group_bytes;
        assert!(
            gap > 0,
            "budget {budget}: a group ({}) must be smaller than the drain gap",
            run.group_bytes
        );
        assert!(
            run.compactions.len() <= turns / gap + 1,
            "budget {budget}: {} compactions over {turns} turns; evicted by each: {:?}",
            run.compactions.len(),
            run.evicted()
        );
        for compaction in &run.compactions {
            assert!(
                compaction.bytes <= compaction.drain,
                "budget {budget}: a compaction kept {} bytes; 60% of the budget is {}",
                compaction.bytes,
                compaction.drain
            );
            assert!(
                compaction.groups >= 1,
                "budget {budget}: the newest group must survive every compaction"
            );
        }
    }
}

/// `older` small groups then one newest group with a `newest_bytes` result,
/// trimmed once; returns the conversation, the newest group and the one
/// compaction the trim reported.
fn trim_once(
    older: usize,
    newest_bytes: usize,
    budget: usize,
) -> (Vec<ChatMessage>, Vec<ChatMessage>, crate::ContextCompaction) {
    let mut messages = conversation();
    for turn in 0..older {
        messages.extend(group(&format!("call-{turn:03}"), "SELECT 1", result(64)));
    }
    let newest = group("call-newest", "SELECT 1", result(newest_bytes));
    messages.extend(newest.clone());
    let log = CompactionLog::default();
    trim_to_budget(
        &mut messages,
        ContextBudget::new(budget, None),
        PROTECTED,
        Some(&log),
        &Default::default(),
    );
    let observed = log.all();
    assert_eq!(observed.len(), 1, "one trim reports one compaction");
    (messages, newest, observed[0])
}

/// A newest group above the drain mark but inside the configured budget is kept
/// whole and alone: the older groups go into the digest, nothing is truncated.
#[test]
fn a_newest_group_above_the_drain_mark_is_kept_whole_and_alone() {
    let (messages, newest, compaction) = trim_once(60, 3 * 1024 * 1024 + 700 * 1024, LARGE_BUDGET);
    assert_eq!(
        messages[messages.len() - 2..],
        newest[..],
        "kept byte-identical"
    );
    assert_eq!(groups(&messages), 1, "only the newest group remains");
    assert_eq!(digests(&messages), 1);
    assert_eq!(compaction.truncated_tool_messages, 0);
}

#[test]
fn a_newest_group_above_the_budget_truncates_only_its_tool_result() {
    let budget = 32 * 1024;
    let (messages, newest, compaction) = trim_once(3, 64 * 1024, budget);
    let (call, reply) = (&messages[messages.len() - 2], &messages[messages.len() - 1]);
    assert_eq!(*call, newest[0], "the newest tool call is not rewritten");
    // A JSON result is rewritten as a valid-JSON envelope, not cut mid-way.
    let parsed: serde_json::Value = serde_json::from_str(&reply.content)
        .expect("a truncated JSON tool result must stay valid JSON");
    assert_eq!(parsed["truncated"], serde_json::json!(true));
    assert_eq!(
        parsed["original_bytes"],
        serde_json::json!(newest[1].content.len())
    );
    assert!(reply.content.len() < newest[1].content.len());
    assert_eq!((groups(&messages), digests(&messages)), (1, 1));
    assert!(size(&messages) <= budget);
    assert_eq!(compaction.evicted_tool_groups, 3);
    assert_eq!(compaction.truncated_tool_messages, 1);
}
