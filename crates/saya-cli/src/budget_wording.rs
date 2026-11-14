//! The one truthful wording for a budget stop, shared by the `ask`
//! classification (exit 6) and the plain REPL's terminal diagnostics.

use crate::render::{RenderFormat, TerminalEvent, render_event};
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

/// Says the truthful partial wording for a settled budget stop as a
/// terminal diagnostic. `None` (no budget stop) says nothing.
pub(crate) fn emit_partial_diagnostic(budget: Option<BudgetDimension>, format: RenderFormat) {
    if let Some(dimension) = budget {
        let rendered = render_event(
            &TerminalEvent::Diagnostic {
                message: budget_partial_message(dimension),
            },
            format,
        );
        print!("{}", rendered.stdout);
        eprint!("{}", rendered.stderr);
    }
}

#[cfg(test)]
#[path = "budget_wording_tests.rs"]
mod tests;
