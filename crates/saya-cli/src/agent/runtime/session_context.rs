// Each argument supplies a distinct context renderer input, so a bundle would
// obscure which session data each block uses.
#[allow(clippy::too_many_arguments)]
fn append_session_context(
    mut context_blocks: Vec<saya_agent::ContextBlock>,
    session: Option<&Arc<SessionUniverse>>,
    prompt: &str,
    registry: &crate::connection::ConnectionRegistry,
    receipt: &crate::contracts::RecallReceipt,
    prior_tool_outcomes: Option<&crate::interactive::session::continuation::PriorToolOutcomes>,
    continuation: Option<&saya_store::StoredContinuation>,
    compaction_summary: Option<&str>,
    context_byte_budget: usize,
) -> (Vec<saya_agent::ContextBlock>, saya_types::SessionTaskList) {
    // The session task list rides the same lane, last: only when non-empty,
    // so an empty list costs zero tokens. The block is the live cell's own
    // rendering (see `session_tasks_render`), quoted as data through the
    // same untrusted lane — never the system prompt, which stays
    // byte-identical whether or not a list exists.
    if let Some(session) = session.as_ref()
        && let Some(block) = crate::interactive::session::tasks::render::render_tasks_block(
            &session.tasks().current(),
        )
    {
        context_blocks.push(block);
    }
    let task_list = session
        .as_ref()
        .map(|universe| universe.tasks().current())
        .unwrap_or_default();
    if session.is_some() {
        if let Some(summary) = compaction_summary
            && let Some(block) = crate::interactive::session::continuation::render_summary_block(
                summary,
                context_byte_budget,
            )
        {
            context_blocks.push(block);
        }
        if let Some(block) = crate::interactive::session::continuation::render_block(
            prompt,
            registry.primary().is_some(),
            registry.dialects(),
            receipt,
            &task_list,
            prior_tool_outcomes,
            context_byte_budget,
        ) {
            context_blocks.push(block);
        }
        context_blocks.extend(
            crate::interactive::session::continuation::render_record_block(
                continuation,
                session
                    .as_ref()
                    .and_then(|session| session.workspace())
                    .as_deref(),
                context_byte_budget,
            ),
        );
    }
    (context_blocks, task_list)
}
