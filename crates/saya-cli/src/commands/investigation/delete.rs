//! `saya investigation delete`: removes the document and its local binding
//! after checking the current revision — a document changed underneath this
//! command is refused, never deleted unseen; a document that cannot be
//! verified is reported, never deleted.

use super::store_failure;
use crate::commands::output::{failure_message, result};
use crate::render::RenderFormat;
use saya_store::InvestigationRepository;

pub(super) fn delete(
    repo: &InvestigationRepository,
    format: RenderFormat,
    id: &str,
    revision: Option<u32>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let id = match super::parse_investigation_id(id) {
        Ok(id) => id,
        Err((code, message)) => return failure_message(code, message, format),
    };
    // The expected revision is the one currently on disk, read first: the
    // repository re-checks it at delete time, so a change between this read
    // and the delete still refuses. An explicit `--revision` narrows the
    // check for scripts that want compare-and-delete.
    let current = match repo.get(&id) {
        Ok(current) => current,
        Err(error) => return store_failure(error, id.as_str(), format),
    };
    if let Some(requested) = revision
        && requested != current.revision
    {
        return failure_message(
            super::EXIT_INVESTIGATION_ERROR,
            format!(
                "investigation {} is at revision {}; pass --revision {} or omit --revision",
                id.as_str(),
                current.revision,
                current.revision
            ),
            format,
        );
    }
    match repo.delete(&id, current.revision) {
        Ok(()) => {}
        Err(error) => return store_failure(error, id.as_str(), format),
    }
    result(
        format!(
            "Deleted investigation {} and its local binding.",
            id.as_str()
        ),
        format,
    )
}
