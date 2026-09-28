//! `saya investigation list` (D2): one bounded page of the saved-investigation
//! collection, with unreadable documents reported as warning lines instead of
//! failing the page.

use crate::commands::output::{failure_message, result};
use crate::render::RenderFormat;
use saya_store::{InvestigationRepository, MAX_LIST_PAGE};

pub(super) fn list(
    repo: &InvestigationRepository,
    format: RenderFormat,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<i32, Box<dyn std::error::Error>> {
    // Default and ceiling are both 50 (invariant 2); zero clamps to one,
    // matching the repository's 1..=50 page bound rather than erroring.
    let limit = limit.unwrap_or(MAX_LIST_PAGE).clamp(1, MAX_LIST_PAGE);
    let offset = offset.unwrap_or(0);
    let page = match repo.list(offset, limit) {
        Ok(page) => page,
        Err(error) => {
            let (code, message) = super::store_error_parts(&error, "");
            return failure_message(code, message, format);
        }
    };
    if page.summaries.is_empty() && page.issues.is_empty() {
        return result("No saved investigations.".to_string(), format);
    }
    let mut lines: Vec<String> = page
        .summaries
        .iter()
        .map(|summary| {
            format!(
                "{}  {}  {}  {}  {}",
                summary.id.as_str(),
                summary.revision,
                summary.dialect.as_str(),
                summary.connection,
                summary.name
            )
        })
        .collect();
    for issue in &page.issues {
        lines.push(format!("warning: {}: {}", issue.file_stem, issue.error));
    }
    if page.capped {
        lines.push("(capped at 500)".to_string());
    }
    result(lines.join("\n"), format)
}
