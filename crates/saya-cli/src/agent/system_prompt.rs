//! System prompt assembly and per-turn context for agent turns — brief the
//! model on its context and memory, and carry the last-SQL hint on the user
//! turn (never the system prompt, where it would perturb the prefix cache).

use super::session_facts::{SessionFacts, session_facts_text};
use crate::connection::ConnectionRegistry;
use crate::plugins::database::prompt::DatabaseFacts;
use saya_agent::AgentMode;
use saya_config::MemoryMode;

/// The database texts this prompt states, re-exported from the one copy in
/// `plugins::database::prompt`: the plugin's `system_section` states the same
/// bytes on a chat run, so neither path can drift from the other.
pub(crate) use crate::plugins::database::prompt::{
    ANSWER_CONTRACT, WORKING_GUIDANCE, memory_section,
};

/// The memory briefing, re-exported for the tests that pin its wording: the
/// assembly reads it through [`memory_section`].
#[cfg(test)]
pub(crate) use crate::plugins::database::prompt::MEMORY_SYSTEM_PROMPT;

/// Plan-mode briefing, appended to the assembled system prompt under Plan
/// only. It states what Plan means — investigate and answer with a plan —
/// and the honest mechanism: write-shaped tools are absent from the model's
/// tool list and would refuse if called. It tells the model not to promise
/// edits as if made, and to describe the change it would make instead.
///
/// The closing sentence is load-bearing: Plan composes with the approval
/// policy and never widens it, so the paragraph claims nothing about reads
/// being unrestricted. What the paragraph *asks* of the model (ending with a
/// plan) is prompt text, not enforcement; what is *enforced* is the hidden
/// definitions, the engine `Deny` on non-read-shaped effects, and the derived
/// write permits being off.
pub(crate) const PLAN_SYSTEM_PROMPT: &str = "You are in Plan mode: investigate and answer with a plan. \
    Write-shaped tools are absent from your tool list and would refuse if called; \
    do not promise edits as if you had made them — describe the change you would make instead. \
    Plan composes with the approval policy and never widens it.";

/// The engine naming and dialect statement for the connected engines. The one
/// copy of the text lives in `plugins::database::prompt`, which is also what the
/// plugin's `system_section` states.
fn naming_section(registry: &ConnectionRegistry) -> Option<String> {
    DatabaseFacts::of(registry).naming_section()
}

pub(crate) use super::turn_context::{last_sql_hint_block, memory_reachable};

/// Assembles the system prompt for a turn from connection registry context,
/// the memory briefing (if assisted and reachable), the session facts (what
/// the session is — connections in scope, bound workspace root — never what
/// the model may do), working guidance, the answer contract, and the engine
/// naming/dialect section.
///
/// This entry point passes an empty session (no workspace root): it keeps
/// the pre-session-facts bytes for callers without a root. The session-aware
/// variant is [`assemble_system_prompt_with_session`]; production threads the
/// session's bound root through it.
///
/// Both are **session-stable**: the same connections, memory mode,
/// reachability, and session facts produce a byte-identical system prompt
/// across turns. The per-turn last-SQL hint is deliberately absent — it rides
/// the user turn as a context block (see [`last_sql_hint_block`]) so it never
/// perturbs the system block a provider's prefix cache is keyed on.
/// Mid-session connection or root changes recompute the block once and pay
/// one cache miss; per-turn volatile content must never enter here.
///
/// This is the Build prompt: the mode-aware entry point is
/// [`assemble_system_prompt_for_mode`], which returns this unchanged under
/// [`AgentMode::Build`] and appends [`PLAN_SYSTEM_PROMPT`] under
/// [`AgentMode::Plan`] — the append is the only difference.
///
/// Three-argument entry point: an empty session (no workspace root) passed
/// into [`assemble_system_prompt_with_session`]. Production always threads
/// the session's bound root through the session-aware entry points; this
/// stays as the documented no-root shape. Called by the empty-session
/// session-facts test, so the no-root shape keeps a direct caller.
pub(crate) fn assemble_system_prompt(
    registry: &ConnectionRegistry,
    memory_mode: MemoryMode,
    memory_reachable: bool,
) -> Option<String> {
    let empty = SessionFacts {
        registry,
        workspace_root: None,
    };
    assemble_system_prompt_with_session(registry, memory_mode, memory_reachable, &empty)
}

/// Session-aware variant of [`assemble_system_prompt`]: the same sections
/// plus the session-facts section (connections in scope, bound workspace
/// root) rendered from `session`. The caller holds `session` stable across
/// the turns of one session — same inputs, byte-identical bytes — so the
/// system block keeps one prefix-cache key.
pub(crate) fn assemble_system_prompt_with_session(
    registry: &ConnectionRegistry,
    memory_mode: MemoryMode,
    memory_reachable: bool,
    session: &SessionFacts<'_>,
) -> Option<String> {
    let base = registry.describe_context();
    let memory = if memory_reachable {
        memory_section(memory_mode)
    } else {
        None
    };

    let mut sections = Vec::new();
    if let Some(b) = base {
        sections.push(b);
    }
    if let Some(m) = memory {
        sections.push(m.to_string());
    }
    if let Some(f) = session_facts_text(session) {
        sections.push(f);
    }
    sections.push(WORKING_GUIDANCE.to_string());
    sections.push(ANSWER_CONTRACT.to_string());
    // Independent of memory mode: schema discovery hands the model a
    // catalog/schema/table tree for every engine, including the ones whose SQL
    // has no such depth, so without this the model writes back the shape it was
    // shown and the statement is rejected. This section also names the engine
    // and pins the dialect (declared types may come from another engine).
    if let Some(n) = naming_section(registry) {
        sections.push(n);
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

/// Assembles the system prompt for a turn under an [`AgentMode`]: the Build
/// prompt from [`assemble_system_prompt`], unchanged, plus — under Plan only —
/// the [`PLAN_SYSTEM_PROMPT`] paragraph appended as its own section. The
/// append is the only difference: under Build the result is byte-identical to
/// [`assemble_system_prompt`]. Mode-aware entry point over
/// [`assemble_system_prompt`]: the Plan-pinned tests read the real assembly
/// through it.
pub(crate) fn assemble_system_prompt_for_mode(
    registry: &ConnectionRegistry,
    memory_mode: MemoryMode,
    memory_reachable: bool,
    session: &SessionFacts<'_>,
    agent_mode: AgentMode,
) -> Option<String> {
    let prompt =
        assemble_system_prompt_with_session(registry, memory_mode, memory_reachable, session)?;
    match agent_mode {
        AgentMode::Build => Some(prompt),
        AgentMode::Plan => Some(format!("{prompt}\n\n{PLAN_SYSTEM_PROMPT}")),
    }
}

/// The interactive session's prompt: [`assemble_system_prompt_for_mode`] plus
/// the local-data shell route, appended at the very end so the prefix stays
/// cacheable. The caller passes the session's own detection, which is
/// [`FileSources::NoData`](crate::agent::file_sources::FileSources::NoData)
/// where the session advertises no `shell` tool.
pub(crate) fn assemble_system_prompt_for_mode_with_file_sources(
    registry: &ConnectionRegistry,
    memory_mode: MemoryMode,
    memory_reachable: bool,
    session: &SessionFacts<'_>,
    agent_mode: AgentMode,
    file_sources: crate::agent::file_sources::FileSources,
) -> Option<String> {
    let prompt = assemble_system_prompt_for_mode(
        registry,
        memory_mode,
        memory_reachable,
        session,
        agent_mode,
    )?;
    let Some(guidance) = crate::agent::file_sources::guidance(file_sources) else {
        return Some(prompt);
    };
    Some(format!("{prompt}\n\n{guidance}"))
}

#[cfg(test)]
#[path = "system_prompt_tests.rs"]
mod tests;
