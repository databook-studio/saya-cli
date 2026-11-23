//! Twin of `agent/system_prompt_tests/plan_mode.rs::
//! plan_prompt_appends_the_paragraph_exactly_once_and_nothing_else`, deleted
//! with the old assembler.
//!
//! The live plan posture is spliced into `DIRECT` by `system`; the existing
//! live test pins its sentences. This pins the two facts that test leaves
//! open: the posture appears once, and removing it leaves `DIRECT` byte for
//! byte — the append is the whole difference.

use super::{DIRECT, PLAN_POSTURE, system};

#[test]
fn the_plan_posture_is_spliced_once_and_nothing_else_changes() {
    assert_eq!(
        system(false, false),
        DIRECT,
        "a job without plan mode gets the direct prompt unchanged"
    );

    let plan = system(true, false);
    assert_eq!(
        plan.matches(PLAN_POSTURE).count(),
        1,
        "the plan paragraph must appear exactly once: {plan}"
    );
    assert_eq!(
        plan.replacen(PLAN_POSTURE, "", 1),
        DIRECT,
        "splicing the posture in must be the only difference"
    );

    let anchor = "You have workspace tools over your own working checkout. ";
    assert!(
        plan.find(anchor).unwrap_or(usize::MAX) < plan.find(PLAN_POSTURE).unwrap_or(0),
        "the posture sits after the workspace sentence: {plan}"
    );
}
