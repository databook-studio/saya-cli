//! The table: today's slash commands, and the one fate each of them has.
//!
//! The mapping itself is the exhaustive `match` in `ChatOps::operation`, so a
//! new `SlashCommand` variant breaks the build rather than landing unmapped.
//! This file pins which fate today's commands take, and that the argument is
//! what decides it where a command both shows and sets (`/model`,
//! `/provider`, `/privacy`).

use super::fixture::{command, harness};
use super::{Fate, Op, OpResult};
use crate::chat::ops_context::{ContextChange, ReadOp};
use saya_agent::ApprovalPolicy;

/// Every spelling of every command variant, and the fate it has.
const TABLE: &[(&str, Fate)] = &[
    ("/start survey the schema", Fate::Session),
    ("/job list", Fate::Session),
    ("/approve 7", Fate::Session),
    ("/connect analytics", Fate::Rollover),
    ("/connections", Fate::ReadOnly),
    ("/include warehouse", Fate::Rollover),
    ("/exclude warehouse", Fate::Rollover),
    ("/provider", Fate::ReadOnly),
    ("/provider anthropic", Fate::Rollover),
    ("/model", Fate::ReadOnly),
    ("/model sonnet", Fate::Rollover),
    ("/privacy", Fate::ReadOnly),
    ("/privacy off", Fate::Rollover),
    ("/approvals ask", Fate::Session),
    ("/mode plan", Fate::Session),
    ("/schema refresh", Fate::ReadOnly),
    ("/sql select 1", Fate::ReadOnly),
    ("/export out.csv", Fate::ReadOnly),
    ("/report out.md", Fate::ReadOnly),
    ("/chart bar", Fate::ReadOnly),
    ("/explain", Fate::ReadOnly),
    ("/clear", Fate::Rollover),
    ("/compact", Fate::Rollover),
    ("/compact --new-session", Fate::Rollover),
    ("/history", Fate::ReadOnly),
    ("/sessions", Fate::ReadOnly),
    ("/sessions list", Fate::ReadOnly),
    ("/sessions delete abc123", Fate::ReadOnly),
    ("/preferences list", Fate::ReadOnly),
    ("/resume abc123", Fate::Rollover),
    ("/rotate", Fate::Rollover),
    ("/setup", Fate::Session),
    ("/reload", Fate::Session),
    ("/columns rental_date", Fate::Session),
    ("/contracts list", Fate::ReadOnly),
    ("/investigation list", Fate::ReadOnly),
    (
        "/run survey the data --allow workspace-write",
        Fate::Unavailable,
    ),
    ("/run cancel abc123", Fate::Unavailable),
    ("/runs", Fate::ReadOnly),
    ("/allow sql:analytics", Fate::Session),
    ("/permissions", Fate::ReadOnly),
    ("/grants", Fate::ReadOnly),
    ("/context", Fate::ReadOnly),
    ("/doctor", Fate::ReadOnly),
    ("/status", Fate::ReadOnly),
    ("/usage", Fate::ReadOnly),
    ("/thinking on", Fate::Session),
    ("/workspace", Fate::ReadOnly),
    ("/help", Fate::ReadOnly),
    ("/tasks", Fate::ReadOnly),
    ("/notes", Fate::ReadOnly),
    ("/recovery", Fate::ReadOnly),
    ("/undo", Fate::Session),
    ("/rewind", Fate::Session),
    ("/rewind 3", Fate::Session),
    ("/exit", Fate::Session),
];

#[test]
fn every_slash_command_maps_to_exactly_one_fate() {
    let harness = harness(ApprovalPolicy::Ask, false);
    let mut wrong = Vec::new();
    for (text, expected) in TABLE {
        let command = command(text);
        let op = harness.ops.operation(&command);
        if op.fate() != *expected {
            wrong.push(format!(
                "{text}: expected {expected:?}, got {:?}",
                op.fate()
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "old commands mapped to the wrong fate:\n{}",
        wrong.join("\n")
    );
    assert_eq!(
        TABLE.len(),
        56,
        "the table names every spelling this unit pins"
    );
}

/// `/grants` is `/permissions`'s alias: one read-only view, two spellings,
/// and the alias carries its own flag so the note can ride the answer.
#[test]
fn permissions_and_grants_are_one_read_operation() {
    let harness = harness(ApprovalPolicy::Ask, false);
    let canonical = harness.ops.operation(&command("/permissions"));
    let alias = harness.ops.operation(&command("/grants"));
    assert_eq!(
        canonical,
        Op::Read(ReadOp::Permissions(
            crate::slash::PermissionsRequest::permissions()
        )),
        "the canonical spelling reads the view"
    );
    assert_eq!(
        alias,
        Op::Read(ReadOp::Permissions(
            crate::slash::PermissionsRequest::grants()
        )),
        "the alias reads the same view, its spelling recorded"
    );
}

#[test]
fn rotate_and_compact_new_session_are_one_operation() {
    let harness = harness(ApprovalPolicy::Ask, false);
    let rotate = harness.ops.operation(&command("/rotate"));
    let compact = harness.ops.operation(&command("/compact --new-session"));
    assert_eq!(
        rotate, compact,
        "one operation, two spellings: the dispatcher produces the same fate for both"
    );
    assert_eq!(rotate, Op::Rollover(ContextChange::Rotate, None));
    assert_eq!(
        harness.ops.operation(&command("/compact")),
        Op::Rollover(ContextChange::Compact, None),
        "bare /compact stays the compaction of this session"
    );
}

#[test]
fn a_bare_context_command_reads_and_a_stated_one_rolls_over() {
    let harness = harness(ApprovalPolicy::Ask, false);
    for (text, read) in [
        ("/provider", ReadOp::Provider),
        ("/model", ReadOp::Model),
        ("/privacy", ReadOp::Privacy),
    ] {
        assert_eq!(
            harness.ops.run(command(text)),
            OpResult::Read(read),
            "{text}: a bare spelling reads"
        );
    }
    for text in ["/provider anthropic", "/model sonnet", "/privacy on"] {
        assert!(
            matches!(harness.ops.run(command(text)), OpResult::Rollover(_)),
            "{text}: a stated spelling requests a rollover"
        );
    }
}

#[test]
fn a_command_not_available_in_chat_carries_todays_wording() {
    let harness = harness(ApprovalPolicy::Ask, false);
    let result = harness
        .ops
        .run(command("/run survey --allow workspace-write"));
    let OpResult::Unavailable(words) = result else {
        panic!("`/run` is not available in a chat session");
    };
    assert!(
        words.contains("saya run"),
        "the refusal points at the command that does run it: {words}"
    );
}
