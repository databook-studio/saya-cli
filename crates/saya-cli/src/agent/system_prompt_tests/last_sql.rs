use super::*;

/// The assembled system prompt must not contain the previous turn's SQL, for
/// any input — the hint now lives on the user turn, so the system block is the
/// same whether or not a query just ran. The session-aware assembly takes no SQL,
/// so the hint prose can never reach it; this locks that property.
#[test]
fn system_prompt_does_not_contain_the_previous_sql() {
    let reg = single_registry("main");
    let prompt =
        assemble_system_prompt_with_session(&reg, MemoryMode::Off, true, &facts(&reg, &None))
            .expect("a prompt");
    assert!(
        !prompt.contains("most recent SQL you ran was"),
        "the SQL hint prose must not appear in the system prompt: {prompt}"
    );
    // The hint block carries the SQL; the system prompt never sees it.
    let hint = last_sql_hint_block("SELECT 1 FROM tbl").expect("a hint block");
    assert!(
        !prompt.contains(&hint.body),
        "the system prompt must not contain the hint block body: {prompt}"
    );
}

/// Two turns that differ only in the previous SQL must produce byte-identical
/// system *messages* (the constant SAYA prompt plus the assembled extra). This
/// is the property the prefix cache depends on, asserted directly through the
/// same `build_messages` the runtime uses, rather than inferred from the hint's
/// absence.
#[test]
fn system_message_is_byte_identical_across_turns_differing_only_in_last_sql() {
    let reg = single_registry("main");
    let system_prompt =
        assemble_system_prompt_with_session(&reg, MemoryMode::Off, true, &facts(&reg, &None));
    let question = "refine the last query";
    // Turn A: no previous SQL. Turn B: a previous SQL hint on the user turn.
    let blocks_a: Vec<saya_agent::ContextBlock> = Vec::new();
    let blocks_b = vec![last_sql_hint_block("SELECT 1 FROM tbl").expect("a hint block")];
    let msgs_a = build_messages(
        system_prompt.as_deref(),
        &blocks_a,
        question,
        &[],
        32 * 1024,
    )
    .unwrap();
    let msgs_b = build_messages(
        system_prompt.as_deref(),
        &blocks_b,
        question,
        &[],
        32 * 1024,
    )
    .unwrap();
    assert_eq!(
        msgs_a[0], msgs_b[0],
        "the system message must not vary with the previous turn's SQL — that is what makes the prefix cache work"
    );
    // Sanity: the user turns do differ (B carries the hint).
    assert_ne!(msgs_a[1], msgs_b[1]);
}

/// The dialect statement must be present and name the connected engine: one
/// SQLite database was a PostgreSQL dump whose DDL still declared `jsonb` and
/// `point`, so the model wrote Postgres syntax SQLite rejected. The prompt must
/// say plainly that the SQL dialect is the connected engine's, whatever the DDL
/// declares.
#[test]
fn dialect_statement_names_the_connected_engine() {
    let prompt = assemble_system_prompt_with_session(
        &registry_with(SqlDialect::Sqlite),
        MemoryMode::Off,
        false,
        &facts(&registry_with(SqlDialect::Sqlite), &None),
    )
    .expect("a prompt");
    assert!(
        prompt.contains("The SQL dialect is sqlite's, whatever the DDL says"),
        "the dialect statement must name the connected engine: {prompt}"
    );
    assert!(
        prompt.contains("declared column types in this database may come from another engine"),
        "the prompt must warn that declared types may come from another engine: {prompt}"
    );
}

/// The hint still reaches the model — on the user turn, never the system
/// message. The block is untrusted data rendered into the user turn by
/// `build_messages`, so the SQL appears beside the question and the system
/// message stays invariant.
#[test]
fn last_sql_hint_reaches_user_turn_not_system_message() {
    let reg = single_registry("main");
    let system_prompt =
        assemble_system_prompt_with_session(&reg, MemoryMode::Off, true, &facts(&reg, &None));
    let hint = last_sql_hint_block("SELECT 1 FROM tbl").expect("a hint block");
    let messages = build_messages(
        system_prompt.as_deref(),
        &[hint],
        "refine it",
        &[],
        32 * 1024,
    )
    .unwrap();
    // The system message is invariant: no hint, no SQL.
    assert_eq!(messages[0].role, "system");
    assert!(
        !messages[0].content.contains("SELECT 1 FROM tbl"),
        "the hint must not leak into the system message: {messages:?}"
    );
    assert!(
        !messages[0].content.contains("most recent SQL you ran was"),
        "the hint prose must not leak into the system message: {messages:?}"
    );
    // The user turn carries the hint, labelled and escaped, before the question.
    assert_eq!(messages[1].role, "user");
    assert!(messages[1].content.contains("SELECT 1 FROM tbl"));
    assert!(messages[1].content.contains("most recent SQL you ran was"));
    assert!(
        messages[1].content.ends_with("refine it"),
        "the user's question must still trail the block: {messages:?}"
    );
}

/// The hint block is `None` for empty/whitespace SQL, so a turn with no prior
/// query adds no block and the user turn is byte-identical to one with no hint.
#[test]
fn last_sql_hint_block_is_none_for_empty_sql() {
    assert!(last_sql_hint_block("").is_none());
    assert!(last_sql_hint_block("   \n\t ").is_none());
    let hint = last_sql_hint_block("SELECT 1").expect("non-empty SQL yields a block");
    assert_eq!(hint.label, LAST_SQL_BLOCK_LABEL);
    assert!(hint.body.contains("SELECT 1"));
    assert!(hint.body.contains("most recent SQL you ran was"));
    assert!(!hint.truncated);
}
