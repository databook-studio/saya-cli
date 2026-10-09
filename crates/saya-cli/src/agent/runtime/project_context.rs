async fn project_turn_context_blocks(
    runtime: &RuntimeConfig,
    session: Option<&Arc<SessionUniverse>>,
    state_db: Option<&SqliteStateStore>,
    prompt: &str,
    system_prompt: Option<&str>,
    mut context_blocks: Vec<saya_agent::ContextBlock>,
) -> Vec<saya_agent::ContextBlock> {
    // Projected instruction sources ride the user turn first — a stable
    // prefix ahead of the volatile hints — as untrusted labelled context
    // blocks, never the system prompt. The remaining budget is what the
    // request still has for instruction blocks after the system prompt, the
    // user's question, and the blocks above, capped by the aggregate
    // instruction ceiling; the fraction reserve keeps history and the answer
    // alive.
    let used_so_far = saya_agent::turn_bytes(system_prompt, &context_blocks, prompt);
    let budget = runtime.resolved.ai.context_byte_budget;
    let remaining = budget
        .saturating_sub(used_so_far)
        .min(super::context::sources::MAX_INSTRUCTION_REQUEST_BYTES)
        .min(
            (budget * super::context::sources::HISTORY_RESERVE_FRACTION.0
                / super::context::sources::HISTORY_RESERVE_FRACTION.1)
                .saturating_sub(used_so_far),
        );
    let projection = super::context::sources::project_instructions(
        &runtime.resolved.instructions,
        session.as_ref().and_then(|session| session.root()),
        &crate::config::sources::user_config_dir(),
        remaining,
    );
    let after_instructions = projection.blocks.len();
    context_blocks.splice(0..0, projection.blocks);
    // Explicit worktree preferences follow the instruction blocks on the same untrusted lane.
    let preferences = super::context::preferences::project_preferences(
        state_db,
        session.as_ref().and_then(|session| session.root()),
        super::context::preferences::remaining_request_bytes(
            saya_agent::turn_bytes(system_prompt, &context_blocks, prompt),
            runtime.resolved.ai.context_byte_budget,
        ),
    )
    .await;
    context_blocks.splice(after_instructions..after_instructions, preferences.blocks);
    context_blocks
}
