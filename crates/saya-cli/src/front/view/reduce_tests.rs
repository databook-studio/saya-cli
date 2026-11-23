//! The reducer's contract, against the two recorded demos: one prompt folds
//! into one message, ONE activity block with a line per tool call, the answer
//! and a result preview — a block keeps its lines when the turn ends, and a
//! failed one never folds at all.

use serde_json::json;

use super::reduce_fixture::{
    ANSWER, PROMPT, activity, answer, completed, count, d14_events, ended, fold, requested, text,
};
use super::{Entry, TranscriptView, apply};

#[test]
fn a_database_turn_is_one_message_one_block_one_answer_one_preview() {
    let view = fold(&d14_events());
    assert_eq!(view.turns().len(), 1, "one prompt is one turn");
    let turn = &view.turns()[0];
    assert_eq!(
        count(turn, |entry| matches!(entry, Entry::UserMessage(_))),
        1,
        "the operator's message opens the turn once: {:?}",
        turn.entries()
    );
    assert_eq!(
        count(turn, |entry| matches!(entry, Entry::Activity(_))),
        1,
        "four tool calls fold into ONE activity block, not one per event: {:?}",
        turn.entries()
    );
    let block = activity(turn);
    assert_eq!(
        block.steps.len(),
        4,
        "one line per tool call: schema discovery and three queries: {:?}",
        block.steps
    );
    assert!(
        block.steps.iter().all(|step| !step.state.is_running()),
        "every step of an ended turn has its outcome: {:?}",
        block.steps
    );
    assert_eq!(
        count(turn, |entry| matches!(entry, Entry::Answer(_))),
        1,
        "the streamed text is ONE answer block: {:?}",
        turn.entries()
    );
    assert_eq!(
        answer(turn),
        ANSWER.concat(),
        "every delta streams into the same answer"
    );
    assert_eq!(
        count(turn, |entry| matches!(entry, Entry::ResultPreview(_))),
        1,
        "the captured query shows as one result preview: {:?}",
        turn.entries()
    );
    assert!(!block.failed, "a turn that answered has no failed step");
    assert!(
        !block.folded,
        "a successful block keeps its step lines once the turn ends — the steps \
         stay readable without a key: {:?}",
        block
    );
}

#[test]
fn a_workspace_turn_folds_the_edit_and_both_commands_into_one_block() {
    let view = fold(&[
        requested("read", json!({"path": "pricing.py"})),
        completed("read", "read 40 lines"),
        requested("edit", json!({"path": "pricing.py"})),
        completed("edit", "pricing.py edited"),
        requested("shell", json!({"program": "python3"})),
        completed("shell", "python3 exited 1"),
        requested("shell", json!({"program": "pytest"})),
        completed("shell", "pytest exited 0"),
        text("The failing test asserted the old total."),
        ended(),
    ]);
    assert_eq!(view.turns().len(), 1);
    let turn = &view.turns()[0];
    assert_eq!(
        count(turn, |entry| matches!(entry, Entry::Activity(_))),
        1,
        "one block for the whole turn: {:?}",
        turn.entries()
    );
    let block = activity(turn);
    let tools: Vec<&str> = block.steps.iter().map(|step| step.tool.as_str()).collect();
    assert_eq!(
        tools,
        ["read", "edit", "shell", "shell"],
        "the block keeps every call in call order: {:?}",
        block.steps
    );
    assert_eq!(
        block
            .steps
            .iter()
            .filter(|step| step.tool == "shell")
            .count(),
        2,
        "both commands are lines of the one block: {:?}",
        block.steps
    );
    assert!(
        !block.failed,
        "a command that exited non-zero is not a failed step: {:?}",
        block.steps
    );
    assert!(
        !block.folded,
        "the block keeps its lines once the turn ends: the commands it ran are \
         why the operator is reading it: {:?}",
        block
    );
    assert_eq!(
        answer(turn),
        "The failing test asserted the old total.",
        "the turn's prose is one answer block"
    );
}

#[test]
fn a_failed_step_marks_the_block_and_it_never_folds() {
    let view = fold(&[
        requested(
            "bounded_sql_query",
            json!({"sql": "SELECT nope FROM orders"}),
        ),
        completed(
            "bounded_sql_query",
            "failed to complete: no such column: nope",
        ),
        ended(),
    ]);
    let block = activity(&view.turns()[0]);
    assert!(
        block.failed,
        "the failed step marks the block: {:?}",
        block.steps
    );
    assert!(
        !block.folded,
        "a failed block keeps its lines — the reason has to stay readable: {:?}",
        block
    );
}

#[test]
fn a_second_turn_opens_its_own_message_and_block() {
    let mut view = TranscriptView::new();
    view.begin_turn(PROMPT);
    for event in [
        requested("schema_discovery", json!({"connection": "demo"})),
        completed("schema_discovery", "discovered 12 tables in demo"),
        text("I'll start by reading the schema."),
        ended(),
    ] {
        apply(&mut view, &event);
    }
    view.begin_turn("and the refunds?");
    for event in [
        requested(
            "bounded_sql_query",
            json!({"sql": "SELECT COUNT(*) FROM refunds"}),
        ),
        completed("bounded_sql_query", "1 row"),
        ended(),
    ] {
        apply(&mut view, &event);
    }
    assert_eq!(view.turns().len(), 2, "the second prompt opens a turn");
    let second = &view.turns()[1];
    assert!(
        matches!(
            second.entries().first(),
            Some(Entry::UserMessage(message)) if message.text == "and the refunds?"
        ),
        "the new turn opens with its own message: {:?}",
        second.entries()
    );
    assert_eq!(
        count(second, |entry| matches!(entry, Entry::Activity(_))),
        1,
        "the new turn has its own activity block: {:?}",
        second.entries()
    );
    assert_eq!(
        activity(second).steps.len(),
        1,
        "the blocks never merge across turns: {:?}",
        activity(second).steps
    );
}
