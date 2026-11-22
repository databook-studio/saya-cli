/// The old interactive turn's entry to the one projection site: the session
/// universe's bound root is what it resolves instructions and preferences
/// against. The projection itself lives beside the two halves it composes
/// (`agent::context::projection`) because the job engine's root turn reads it
/// too.
async fn project_turn_context_blocks(
    runtime: &RuntimeConfig,
    session: Option<&Arc<SessionUniverse>>,
    state_db: Option<&SqliteStateStore>,
    prompt: &str,
    system_prompt: Option<&str>,
    context_blocks: Vec<saya_agent::ContextBlock>,
) -> Vec<saya_agent::ContextBlock> {
    super::context::projection::project_untrusted_context(
        runtime,
        session.and_then(|session| session.root()),
        state_db,
        prompt,
        system_prompt,
        context_blocks,
    )
    .await
}
