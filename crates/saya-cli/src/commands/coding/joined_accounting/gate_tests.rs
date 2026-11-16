use super::only_unknown_tool_calls_remain;
use saya_harness::coding::budget::BudgetSnapshot;

#[test]
fn unknown_model_request_usage_does_not_stop_historical_tool_calls_being_proven() {
    let budget = BudgetSnapshot {
        unknown_tool_calls: 1,
        unknown_transport_attempts: 2,
        unknown_logical_requests: 1,
        ..BudgetSnapshot::default()
    };
    assert!(only_unknown_tool_calls_remain(&budget));
}

#[test]
fn another_unknown_effect_or_live_work_still_stops_it() {
    let tools = BudgetSnapshot {
        unknown_tool_calls: 1,
        ..BudgetSnapshot::default()
    };
    let cases = [
        ("no unknown tool call", BudgetSnapshot::default()),
        (
            "held model slot",
            BudgetSnapshot {
                model_slots_active: 1,
                ..tools.clone()
            },
        ),
        (
            "active command",
            BudgetSnapshot {
                command_slots_active: 1,
                ..tools.clone()
            },
        ),
        (
            "poisoned owner",
            BudgetSnapshot {
                poisoned: true,
                ..tools.clone()
            },
        ),
    ];
    for (name, budget) in cases {
        assert!(!only_unknown_tool_calls_remain(&budget), "{name}");
    }
}
