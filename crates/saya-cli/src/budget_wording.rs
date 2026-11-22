//! The one truthful wording for a budget stop, shared by the `ask`
//! classification (exit 6) and the plain REPL's terminal diagnostics.

use saya_agent::{BudgetDimension, CancellationReason};

/// The budget dimension a settled turn stopped on, if any: the question
/// token's first stop reason decides, so a later stop request cannot
/// overwrite an earlier budget stop.
pub(crate) fn budget_dimension(reason: Option<CancellationReason>) -> Option<BudgetDimension> {
    match reason {
        Some(CancellationReason::Budget(dimension)) => Some(dimension),
        _ => None,
    }
}

/// The dimension the request preflight refuses on, when a turn's first stop
/// reason is the one the row names: the model window.
///
/// A preflight refusal is latched on the turn's token by the engine before any
/// request is admitted, and the turn comes back as partial output rather than as
/// an error — so it is the turn's own stop the surface has to read, and it is a
/// budget stop like the configured dimensions. Every other budget dimension on
/// that token belongs to an owner that already states it (the plugin's armed
/// policy, the question's own scope); the window has no other reader.
pub(crate) fn preflight_dimension(reason: Option<CancellationReason>) -> Option<BudgetDimension> {
    budget_dimension(reason).filter(|dimension| *dimension == BudgetDimension::RequestContextWindow)
}

/// The fixed truthful wording for a budget stop: the run stopped on its
/// configured budget, so whatever answer it produced is partial or absent.
pub(crate) fn budget_partial_message(dimension: BudgetDimension) -> String {
    let what = match dimension {
        BudgetDimension::LogicalRequests => "logical-requests".to_string(),
        BudgetDimension::RequestedToolCalls => "requested-tool-calls".to_string(),
        BudgetDimension::ElapsedTime => "elapsed-time".to_string(),
        BudgetDimension::ReportedTokens => "reported-tokens".to_string(),
        BudgetDimension::DownloadBytes => "download-bytes".to_string(),
        // A dimension added later keeps the truthful class and shape; its
        // debug name stands in for the words it does not have yet.
        other => format!("{other:?}").to_lowercase(),
    };
    format!("the run stopped on its {what} budget; any answer it produced is partial or absent")
}

#[cfg(test)]
#[path = "budget_wording_tests.rs"]
mod tests;
