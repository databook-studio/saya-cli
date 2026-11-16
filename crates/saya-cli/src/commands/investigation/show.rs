//! `saya investigation show`: the exact stored definition plus the local
//! review binding for this machine — with the opaque profile identity never
//! printed, in any format.

use super::store_failure;
use crate::commands::output::{failure_message, result};
use crate::render::RenderFormat;
use saya_store::InvestigationRepository;

pub(super) fn show(
    repo: &InvestigationRepository,
    format: RenderFormat,
    id: &str,
) -> Result<i32, Box<dyn std::error::Error>> {
    let id = match super::parse_investigation_id(id) {
        Ok(id) => id,
        Err((code, message)) => return failure_message(code, message, format),
    };
    let definition = match repo.get(&id) {
        Ok(definition) => definition,
        Err(error) => return store_failure(error, id.as_str(), format),
    };
    // An unreadable binding is reported, not collapsed onto "none" — claiming
    // none would invite a re-save that quietly drops the review record.
    let binding_line = match repo.get_binding(&id) {
        Ok(Some(binding)) => format!(
            "local binding: {} (reviewed revision {})",
            binding.profile, binding.reviewed_revision
        ),
        Ok(None) => "local binding: none".to_string(),
        Err(error) => format!("local binding: unreadable ({error:?})"),
    };
    let json = definition.to_json_pretty()?;
    result(format!("{json}\n{binding_line}"), format)
}
