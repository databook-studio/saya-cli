use crate::render::{RenderFormat, ResumeEffectCode};
use saya_harness::engine::ResumeEffect;
use saya_types::RunId;

const MAX_RESUME_RUN_ID_CHARS: usize = 64;
const MAX_RESUME_GOAL_CHARS: usize = 160;

pub(super) fn emit(
    run_id: &RunId,
    step: usize,
    goal: &str,
    effects: Vec<ResumeEffect>,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    crate::commands::output::resume_uncertain(
        bounded_run_id(run_id),
        step,
        bounded_redacted_goal(goal),
        effects.into_iter().map(effect_code).collect(),
        format,
    )
}

fn bounded_run_id(run_id: &RunId) -> String {
    run_id
        .as_str()
        .chars()
        .take(MAX_RESUME_RUN_ID_CHARS)
        .collect()
}

fn bounded_redacted_goal(goal: &str) -> String {
    let redacted = saya_types::redact(goal);
    let mut bounded = redacted
        .chars()
        .take(MAX_RESUME_GOAL_CHARS)
        .collect::<String>();
    if redacted.chars().count() > MAX_RESUME_GOAL_CHARS {
        bounded.push('…');
    }
    bounded
}

fn effect_code(effect: ResumeEffect) -> ResumeEffectCode {
    use ResumeEffect as E;
    match effect {
        E::WorkspaceWrite => ResumeEffectCode::WorkspaceWrite,
        E::Scratch => ResumeEffectCode::Scratch,
        E::Runner => ResumeEffectCode::Runner,
        E::Interpreter => ResumeEffectCode::Interpreter,
        E::Fetch => ResumeEffectCode::Fetch,
        E::ExternalSideEffect => ResumeEffectCode::ExternalSideEffect,
        E::LocalStateWrite => ResumeEffectCode::LocalStateWrite,
        E::Unknown => ResumeEffectCode::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncertainty_goal_is_redacted_and_bounded() {
        let raw = format!("token=private {}", "g".repeat(300));
        let goal = bounded_redacted_goal(&raw);
        assert!(goal.contains("token=[redacted]"));
        assert!(!goal.contains("private"));
        assert_eq!(goal.chars().count(), MAX_RESUME_GOAL_CHARS + 1);
        assert!(goal.ends_with('…'));
    }
}
