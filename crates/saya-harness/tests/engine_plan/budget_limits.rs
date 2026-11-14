use super::*;

/// One budget dimension, widened in turn.
#[derive(Debug, Clone, Copy)]
enum Dimension {
    WallClock,
    DownloadedBytes,
    WorkspaceBytes,
    WorkspaceFiles,
    ProcessCount,
    ProcessTime,
    Turns,
    ToolCalls,
    TokensPerEndpoint,
}

const DIMENSIONS: &[Dimension] = &[
    Dimension::WallClock,
    Dimension::DownloadedBytes,
    Dimension::WorkspaceBytes,
    Dimension::WorkspaceFiles,
    Dimension::ProcessCount,
    Dimension::ProcessTime,
    Dimension::Turns,
    Dimension::ToolCalls,
    Dimension::TokensPerEndpoint,
];

/// A budgets object carrying exactly one declared ceiling.
fn ceilings(dimension: Dimension, value: u64) -> Budgets {
    let mut ceilings = Budgets::default();
    match dimension {
        Dimension::WallClock => ceilings.wall_clock = Some(Duration::from_secs(value)),
        Dimension::DownloadedBytes => ceilings.downloaded_bytes = Some(value),
        Dimension::WorkspaceBytes => ceilings.workspace_bytes = Some(value),
        Dimension::WorkspaceFiles => ceilings.workspace_files = Some(value),
        Dimension::ProcessCount => ceilings.process_count = Some(value),
        Dimension::ProcessTime => ceilings.process_time = Some(Duration::from_secs(value)),
        Dimension::Turns => ceilings.turns = Some(value),
        Dimension::ToolCalls => ceilings.tool_calls = Some(value),
        Dimension::TokensPerEndpoint => {
            ceilings.tokens_per_endpoint.insert("primary".into(), value);
        }
    }
    ceilings
}

/// A step budget exceeding the run budget is rejected on each dimension in
/// turn — the run budget is the remaining budget at binding time, so a step
/// that widens any one of them cannot bind.
#[tokio::test]
async fn a_step_budget_exceeding_the_run_budget_is_rejected_on_every_dimension() {
    for dimension in DIMENSIONS {
        let run_ceilings = ceilings(*dimension, 10);
        let step_ceilings = ceilings(*dimension, 11);
        let step = format!(
            r#"{{"goal": "spend past the run", "capabilities": {{}}, "budget": {}, "expects": [], "endpoint": null}}"#,
            serde_json::to_string(&step_ceilings).unwrap()
        );
        let planner = ScriptedPlanner::new(vec![plan_json(&step); 3]);
        let driver = PlanDriver::new(&planner, planner_request("g"));

        let error = driver
            .propose(&Capabilities::default(), &run_ceilings)
            .await
            .unwrap_err();

        assert!(
            matches!(
                &error,
                PlanError::Exhausted {
                    attempts: 3,
                    last: PlanRejection::BudgetTooWide { step: 0 },
                }
            ),
            "dimension {dimension:?} must refuse a widening step budget: {error:?}"
        );
    }
}
