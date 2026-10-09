use super::*;

#[test]
fn memory_section_appears_under_assisted_and_absent_under_off() {
    assert!(memory_section(MemoryMode::Assisted).is_some());
    assert!(memory_section(MemoryMode::Off).is_none());

    let text = memory_section(MemoryMode::Assisted).unwrap();
    // Briefing contents verification
    assert!(text.contains("durable knowledge"));
    assert!(text.contains("Confirmed facts relevant to the question are already supplied"));
    assert!(text.contains("contract_search"));
    assert!(text.contains("contract_read"));
    assert!(text.contains("restate it explicitly and precisely in the answer"));
    // The naming rule is not here: its correct wording depends on the engine,
    // so it is built per connection alongside this section.
    assert!(!text.contains("catalog.schema.object"));
}

#[test]
fn assemble_system_prompt_single_connection_off() {
    let reg = single_registry("main");
    let prompt =
        assemble_system_prompt_with_session(&reg, MemoryMode::Off, true, &facts(&reg, &None))
            .expect("a prompt");
    // With one connection and memory off there is no context and no briefing —
    // but the model is still shown a catalog/schema/table tree by schema
    // discovery, so it still has to be told what SQL will accept.
    assert!(!prompt.contains("durable knowledge"));
    assert!(prompt.contains("catalog.schema.object"));
}

#[test]
fn assemble_system_prompt_single_connection_assisted() {
    let reg = single_registry("main");
    let prompt =
        assemble_system_prompt_with_session(&reg, MemoryMode::Assisted, true, &facts(&reg, &None))
            .expect("a prompt");
    assert!(prompt.starts_with(MEMORY_SYSTEM_PROMPT));
    // A single PostgreSQL connection: the memory briefing, then its naming rule.
    assert!(prompt.contains("catalog.schema.object"));
}

/// The previous turn's SQL no longer reaches the system prompt; it rides the
/// user turn as a context block (see [`last_sql_hint_reaches_user_turn`]). With
/// memory off, the system prompt carries no hint and no SQL text.
#[test]
fn assemble_system_prompt_with_last_sql_and_memory_off() {
    let reg = single_registry("main");
    let prompt =
        assemble_system_prompt_with_session(&reg, MemoryMode::Off, true, &facts(&reg, &None))
            .expect("a prompt");
    assert!(!prompt.contains("durable knowledge"));
    assert!(
        !prompt.contains("most recent SQL you ran was"),
        "the hint must not appear in the system prompt: {prompt}"
    );
}

/// The hint is absent from the system prompt under assisted memory too — the
/// hint block is built separately and attached to the user turn.
#[test]
fn assemble_system_prompt_with_last_sql_and_assisted() {
    let reg = single_registry("main");
    let prompt =
        assemble_system_prompt_with_session(&reg, MemoryMode::Assisted, true, &facts(&reg, &None))
            .expect("a prompt");
    assert!(prompt.contains(MEMORY_SYSTEM_PROMPT));
    assert!(
        !prompt.contains("most recent SQL you ran was"),
        "the hint must not appear in the system prompt: {prompt}"
    );
}

/// The multi-connection system prompt keeps its section order (connections,
/// memory, guidance) and never carries the previous SQL.
#[test]
fn assemble_system_prompt_multi_connection_assisted_and_sql() {
    let reg = multi_registry();
    let prompt =
        assemble_system_prompt_with_session(&reg, MemoryMode::Assisted, true, &facts(&reg, &None))
            .expect("a prompt");

    // Contains all three sections in expected order.
    let conn_idx = prompt.find("Available database connections").unwrap();
    let mem_idx = prompt.find("SAYA maintains durable knowledge").unwrap();
    let guidance_idx = prompt.find("Multi-step work is expected").unwrap();

    assert!(conn_idx < mem_idx);
    assert!(mem_idx < guidance_idx);
    assert!(
        !prompt.contains("most recent SQL you ran was"),
        "the hint must not appear in the system prompt: {prompt}"
    );
    // The dialect statement rides every request, multi-connection included: the
    // engines are listed, then the dialect is pinned whatever the DDL declares.
    assert!(
        prompt.contains("The SQL dialect is the connected engine's, whatever the DDL says"),
        "the dialect statement must appear on every request: {prompt}"
    );
    assert!(
        prompt.contains("- postgresql: `catalog.schema.object`"),
        "the multi-connection list must name each engine: {prompt}"
    );
}

#[test]
fn assemble_system_prompt_stays_within_budget() {
    let reg = multi_registry();
    let prompt =
        assemble_system_prompt_with_session(&reg, MemoryMode::Assisted, true, &facts(&reg, &None));
    assert!(prompt.is_some());
    let system_text = prompt.unwrap();

    // Check raw length is tiny compared to a conversation budget.
    assert!(system_text.len() < 4000);

    // The system prompt plus an ordinary question is far below any realistic
    // context_byte_budget — the loop bounds the conversation, not a start-of-run
    // cap, so this is a sanity check, not an enforced ceiling.
    let bytes = turn_bytes(Some(&system_text), &[], "How many orders were placed?");
    assert!(bytes < 32 * 1024);
}

/// Assisted, but the state store did not open or the privacy gate is shut: the
/// section promises supplied facts and two tools that this turn does not have,
/// so it must not appear.
#[test]
fn memory_section_is_absent_when_memory_is_configured_on_but_unreachable() {
    let reg = single_registry("main");
    let prompt =
        assemble_system_prompt_with_session(&reg, MemoryMode::Assisted, false, &facts(&reg, &None))
            .expect("a prompt");
    assert!(
        !prompt.contains("durable knowledge"),
        "memory is unreachable, so the briefing must not claim otherwise: {prompt}"
    );

    assert!(memory_reachable(true, true));
    assert!(
        !memory_reachable(false, true),
        "no state store, no briefing"
    );
    assert!(
        !memory_reachable(true, false),
        "privacy gate shut, no briefing"
    );
}
