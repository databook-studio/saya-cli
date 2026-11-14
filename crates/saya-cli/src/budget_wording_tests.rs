use super::budget_dimension;
use super::budget_partial_message;
use saya_agent::{BudgetDimension, CancellationReason};

#[test]
fn budget_partial_message_names_each_dimension_truthfully() {
    let cases = [
        (BudgetDimension::LogicalRequests, "logical-requests"),
        (BudgetDimension::RequestedToolCalls, "requested-tool-calls"),
        (BudgetDimension::ElapsedTime, "elapsed-time"),
        (BudgetDimension::ReportedTokens, "reported-tokens"),
        (BudgetDimension::DownloadBytes, "download-bytes"),
    ];
    for (dimension, what) in cases {
        assert_eq!(
            budget_partial_message(dimension),
            format!(
                "the run stopped on its {what} budget; any answer it produced is partial or absent"
            )
        );
    }
}

#[test]
fn budget_dimension_reads_only_budget_stops() {
    assert_eq!(
        budget_dimension(Some(CancellationReason::Budget(
            BudgetDimension::ElapsedTime
        ))),
        Some(BudgetDimension::ElapsedTime)
    );
    assert_eq!(budget_dimension(Some(CancellationReason::User)), None);
    assert_eq!(
        budget_dimension(Some(CancellationReason::OwnerFailure)),
        None
    );
    assert_eq!(budget_dimension(None), None);
}
