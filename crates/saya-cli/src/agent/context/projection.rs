//! The one site the request's untrusted project context enters at.
//!
//! It lives beside the two halves it composes — [`sources`] and
//! [`preferences`] — rather than in the turn body that first called it: the
//! job engine's root turn reads it too (`commands::coding::parent_loop`), so
//! the projection belongs to the context module both surfaces read.

use crate::config::runtime::RuntimeConfig;
use saya_store::SqliteStateStore;

/// The projected instruction sources first, then the worktree's explicit
/// preferences, as labelled blocks on the user turn — never the system prompt.
/// `root` is the workspace root both are resolved against; `None` binds
/// nothing and adds no block, and so does a root whose preferences cannot be
/// read.
///
/// Two surfaces state it: a chat run's root turn, and the old interactive
/// turn's own entry (`runtime::project_context`), over the session universe's
/// bound root.
pub(crate) async fn project_untrusted_context(
    runtime: &RuntimeConfig,
    root: Option<&std::path::Path>,
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
        .min(super::sources::MAX_INSTRUCTION_REQUEST_BYTES)
        .min(
            (budget * super::sources::HISTORY_RESERVE_FRACTION.0
                / super::sources::HISTORY_RESERVE_FRACTION.1)
                .saturating_sub(used_so_far),
        );
    let projection = super::sources::project_instructions(
        &runtime.resolved.instructions,
        root,
        std::env::current_dir().ok().as_deref(),
        &crate::config::sources::user_config_dir(),
        remaining,
    );
    let after_instructions = projection.blocks.len();
    context_blocks.splice(0..0, projection.blocks);
    // Explicit worktree preferences follow the instruction blocks on the same untrusted lane.
    let preferences = super::preferences::project_preferences(
        state_db,
        root,
        super::preferences::remaining_request_bytes(
            saya_agent::turn_bytes(system_prompt, &context_blocks, prompt),
            runtime.resolved.ai.context_byte_budget,
        ),
    )
    .await;
    context_blocks.splice(after_instructions..after_instructions, preferences.blocks);
    context_blocks
}
