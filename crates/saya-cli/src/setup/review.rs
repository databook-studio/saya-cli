//! Renders the review the user confirms before anything is written: every
//! file that will be created or appended — for an append, only the appended
//! block, never the whole file — and every note the engine attached.

use std::path::Path;

use super::{PlannedWrite, SetupPlan};

/// The review text: the exact planned content per file, then the notes.
pub(crate) fn render(dir: &Path, planned: &SetupPlan) -> String {
    let mut text = String::from("The following changes will be written:\n");
    for write in &planned.writes {
        let action = if write.created { "create" } else { "append to" };
        text.push_str(&format!(
            "── {action} {} ──\n",
            dir.join(&write.file).display()
        ));
        text.push_str(&shown_text(dir, write));
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push('\n');
    }
    for note in &planned.notes {
        text.push_str("note: ");
        text.push_str(note);
        text.push('\n');
    }
    text
}

/// For an append, only the appended block: the plan guarantees the existing
/// bytes are an exact prefix of the planned content, so stripping the
/// on-disk prefix leaves the block. If the file changed (or vanished) since
/// planning, the full planned text is shown instead — never less than the
/// truth.
fn shown_text(dir: &Path, planned: &PlannedWrite) -> String {
    if planned.created {
        return planned.content.clone();
    }
    match std::fs::read(dir.join(&planned.file)) {
        Ok(existing) if planned.content.as_bytes().starts_with(existing.as_slice()) => {
            String::from_utf8(planned.content.as_bytes()[existing.len()..].to_vec())
                .unwrap_or_else(|_| planned.content.clone())
        }
        _ => planned.content.clone(),
    }
}
