//! `saya investigation import` (D5): reads a portable definition file,
//! validates the whole document, re-gates the SQL exactly as save does, and
//! stores it with no local binding. Nothing is executed and no connection is
//! made — a provider is never constructed, and the imported document carries
//! no review authority until a local run binds it.

use std::fs;
use std::io::Read;
use std::path::Path;

use super::objects;
use super::{EXIT_INVESTIGATION_ERROR, EXIT_SAFETY, store_failure};
use crate::commands::output::{failure_message, result};
use crate::render::RenderFormat;
use saya_connectors::prepare_for_dialect;
use saya_store::{InvestigationRepository, StoreError};
use saya_types::investigation::{InvestigationDefinitionV1, MAX_DEFINITION_BYTES};
use saya_types::redact;

/// What a fresh import says when it succeeds: the stored alias is not a
/// binding, and nothing ran.
const IMPORTED_NOTE: &str = "Imported without a local connection. Run with --connection <profile> to map it; nothing was executed.";
/// The idempotent re-import: the document is already stored and identical.
const ALREADY_PRESENT_NOTE: &str = "Already present and identical; nothing changed. Run with --connection <profile> to map it; nothing was executed.";

/// The import flow: read and fully validate the file, re-gate the SQL,
/// store it (idempotent for identical content, a conflict otherwise), and
/// print the preview.
pub(super) fn import(
    repo: &InvestigationRepository,
    format: RenderFormat,
    path: &Path,
) -> Result<i32, Box<dyn std::error::Error>> {
    let definition = match read_definition(path) {
        Ok(definition) => definition,
        Err((code, message)) => return failure_message(code, message, format),
    };
    // The execution-time gate, without execution — the same gate save uses,
    // because a hand-edited file can carry SQL the document bounds accept.
    if let Err(error) = prepare_for_dialect(&definition.sql, 1, definition.dialect) {
        return failure_message(EXIT_SAFETY, error.to_string(), format);
    }
    if redact(&definition.sql) != definition.sql {
        return failure_message(
            EXIT_INVESTIGATION_ERROR,
            "SQL contains credential-shaped text; saved SQL is kept exactly, so remove it"
                .to_string(),
            format,
        );
    }
    // The stored objects field is informational only (A2): it must agree
    // with what the SQL actually references, recomputed here, so a document
    // can never lie about — or empty out — its own review dependencies.
    let recomputed = objects::canonical_objects(&definition.sql, definition.dialect);
    if definition.objects != recomputed {
        return failure_message(
            EXIT_INVESTIGATION_ERROR,
            "objects do not match the SQL; re-export the investigation".to_string(),
            format,
        );
    }
    match repo.get(&definition.id) {
        Ok(existing) if existing == definition => {
            return result(preview(&definition, ALREADY_PRESENT_NOTE), format);
        }
        Ok(_) => {
            return failure_message(
                EXIT_INVESTIGATION_ERROR,
                format!(
                    "an investigation with id {} already exists with different content; delete it or import a different document",
                    definition.id.as_str()
                ),
                format,
            );
        }
        Err(StoreError::NotFound) => {}
        Err(error) => return store_failure(error, definition.id.as_str(), format),
    }
    if let Err(error) = repo.create(&definition) {
        // A conflict here is a raced concurrent import; same refusal.
        let id = definition.id.as_str();
        if matches!(error, StoreError::Conflict) {
            return failure_message(
                EXIT_INVESTIGATION_ERROR,
                format!(
                    "an investigation with id {id} already exists with different content; delete it or import a different document"
                ),
                format,
            );
        }
        return store_failure(error, id, format);
    }
    result(preview(&definition, IMPORTED_NOTE), format)
}

/// Reads at most `MAX_DEFINITION_BYTES + 1` bytes from a regular file, then
/// runs the full `from_json_bytes` validation. Every refusal is a
/// `(exit code, message)` pair whose text never echoes SQL.
fn read_definition(path: &Path) -> Result<InvestigationDefinitionV1, (i32, String)> {
    let display = path.display();
    let not_readable = |error: std::io::Error| {
        (
            EXIT_INVESTIGATION_ERROR,
            format!("could not read {display}: {error}"),
        )
    };
    // Checked before opening, so a FIFO or device is refused rather than
    // opened (opening either would block). A symlink resolving to a regular
    // file reads exactly that file, which is the file being vetted.
    let meta = fs::metadata(path).map_err(not_readable)?;
    if !meta.is_file() {
        return Err((
            EXIT_INVESTIGATION_ERROR,
            format!("{display} is not a regular file"),
        ));
    }
    let file = fs::File::open(path).map_err(not_readable)?;
    let mut bytes = Vec::new();
    file.take(MAX_DEFINITION_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(not_readable)?;
    if bytes.len() > MAX_DEFINITION_BYTES {
        return Err((
            EXIT_INVESTIGATION_ERROR,
            format!("{display} is over the {MAX_DEFINITION_BYTES}-byte investigation limit"),
        ));
    }
    InvestigationDefinitionV1::from_json_bytes(&bytes)
        .map_err(|error| (EXIT_INVESTIGATION_ERROR, error.to_string()))
}

/// The preview: id, name, dialect, the connection requirement, the objects,
/// and the exact SQL — followed by the outcome note.
fn preview(definition: &InvestigationDefinitionV1, note: &str) -> String {
    let objects = if definition.objects.is_empty() {
        "none".to_string()
    } else {
        definition.objects.join(", ")
    };
    format!(
        "id: {}\nname: {}\ndialect: {}\nconnection: requires --connection <profile> to run (saved alias \"{}\")\nobjects: {objects}\nsql:\n{}\n\n{note}",
        definition.id.as_str(),
        definition.name,
        definition.dialect.as_str(),
        definition.connection,
        definition.sql,
    )
}
