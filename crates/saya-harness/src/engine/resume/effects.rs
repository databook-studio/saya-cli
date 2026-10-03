use saya_agent::LocalStateEffect;
use saya_types::StepSpec;

use super::ResumeEffect;
use crate::engine::episode::StepToolset;

/// Infer uncertainty from declared capabilities and actual per-step tool
/// contracts. Approval and `read_only` flags do not establish replay safety;
/// database observations with no local or external write remain fresh reads.
pub(super) fn possible_effects(
    step: &StepSpec,
    toolset: Option<&StepToolset>,
) -> Vec<ResumeEffect> {
    let mut effects = Vec::new();
    let capabilities = &step.capabilities;
    if capabilities.workspace_write {
        effects.push(ResumeEffect::WorkspaceWrite);
    }
    if capabilities.scratch {
        effects.push(ResumeEffect::Scratch);
    }
    if capabilities.runner.is_some() {
        effects.push(ResumeEffect::Runner);
    }
    if capabilities.interpreter.is_some() {
        effects.push(ResumeEffect::Interpreter);
    }
    if capabilities.fetch.is_some() {
        effects.push(ResumeEffect::Fetch);
    }
    let Some(toolset) = toolset else {
        push_unique(&mut effects, ResumeEffect::Unknown);
        return effects;
    };
    for definition in &toolset.definitions {
        if definition.effect.external_side_effect {
            push_unique(&mut effects, ResumeEffect::ExternalSideEffect);
        }
        match definition.effect.local_state {
            LocalStateEffect::WriteWorkspace => {
                push_unique(&mut effects, ResumeEffect::WorkspaceWrite);
            }
            LocalStateEffect::WriteCandidate | LocalStateEffect::WriteSession => {
                push_unique(&mut effects, ResumeEffect::LocalStateWrite);
            }
            LocalStateEffect::None | LocalStateEffect::Read => {}
            _ => push_unique(&mut effects, ResumeEffect::Unknown),
        }
    }
    effects
}

fn push_unique(effects: &mut Vec<ResumeEffect>, effect: ResumeEffect) {
    if !effects.contains(&effect) {
        effects.push(effect);
    }
}
