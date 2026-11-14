use super::blocks_follow_up;
use saya_harness::coding::budget::BudgetSnapshot;

#[test]
fn unknown_model_request_usage_does_not_block_a_follow_up() {
    let budget = BudgetSnapshot {
        unknown_transport_attempts: 3,
        unknown_logical_requests: 1,
        ..BudgetSnapshot::default()
    };
    assert!(!blocks_follow_up(&budget));
}

#[test]
fn an_unsettled_effect_or_live_work_still_blocks_a_follow_up() {
    let cases = [
        (
            "unsettled command",
            BudgetSnapshot {
                command_slots_active: 1,
                ..BudgetSnapshot::default()
            },
        ),
        (
            "active child",
            BudgetSnapshot {
                child_slots_active: 1,
                ..BudgetSnapshot::default()
            },
        ),
        (
            "poisoned owner",
            BudgetSnapshot {
                poisoned: true,
                ..BudgetSnapshot::default()
            },
        ),
    ];
    for (name, budget) in cases {
        assert!(blocks_follow_up(&budget), "{name}");
    }
}
