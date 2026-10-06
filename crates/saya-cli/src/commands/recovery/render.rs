//! Plain-text rendering for the recovery listing: a quota header, one
//! line per recorded edit, a pinned sentence for every restore status,
//! and the opt-in explanation for an empty session. No digests or file
//! content appear in any text except the bounded diff itself.

use super::render_outcome::restore_refusal_text;
use super::{BeforeSummary, ChangesView, RestoreRefusal, StatusNow};
use crate::interactive::tui::replay::relative_time;
use saya_types::{CheckpointState, EditSource, FileVersion, RestoreEligibility};

/// Renders the listing: a quota header, then one line per recorded edit.
/// An empty session renders the opt-in explanation instead.
pub(crate) fn render_changes(view: &ChangesView, now_ms: i64) -> String {
    if view.items.is_empty() {
        return "No recorded edits. Recovery records file edits only while it is on (opt-in); \
                it covers workspace_write/workspace_edit — never commands, downloads, \
                scratch or database effects."
            .to_owned();
    }
    let mut lines = vec![format!(
        "Recorded edits this session ({} of {} · {} KiB of {} MiB):",
        view.usage.edits,
        view.max_edits,
        view.usage.retained_bytes.div_ceil(1024),
        view.max_bytes / (1024 * 1024),
    )];
    for item in &view.items {
        let age = relative_time((now_ms - item.created_unix_ms).max(0) as u128);
        lines.push(format!(
            "{} {} {} {} {} -> {} · {} · {}",
            item.id,
            item.path,
            source_label(&item.source),
            state_label(&item.state),
            before_label(item.before),
            after_label(item.after.as_ref()),
            age,
            list_status_text(&item.restore, &item.path),
        ));
    }
    lines.join("\n")
}

/// The listing status: restorable edits say so; every other status reuses
/// the restore refusal sentence for the same state, so a list and a
/// refused restore can never disagree about why.
fn list_status_text(status: &StatusNow, path: &str) -> String {
    match status {
        StatusNow::Known(RestoreEligibility::Eligible) => {
            "restorable now: the file still matches the recorded edit".to_owned()
        }
        StatusNow::Known(_) => restore_refusal_text(&RestoreRefusal::NotEligible {
            status: status.clone(),
            path: path.to_owned(),
        }),
        StatusNow::Unreadable => {
            "cannot read the file now, so its restore status is unknown".to_owned()
        }
        StatusNow::NotChecked => {
            "restore status not checked (the listing refreshes at most 32 files)".to_owned()
        }
    }
}

fn source_label(source: &EditSource) -> &'static str {
    match source {
        EditSource::WorkspaceEdit => "workspace_edit",
        EditSource::WorkspaceWrite => "workspace_write",
        _ => "unknown source",
    }
}

fn state_label(state: &CheckpointState) -> &'static str {
    match state {
        CheckpointState::Prepared => "prepared",
        CheckpointState::Committed => "committed",
        CheckpointState::Uncertain => "uncertain",
        _ => "unknown state",
    }
}

fn before_label(before: BeforeSummary) -> String {
    match before {
        BeforeSummary::Absent => "absent".to_owned(),
        BeforeSummary::Present { size } => format!("present {size} B"),
    }
}

fn after_label(after: Option<&FileVersion>) -> String {
    match after {
        None => "absent".to_owned(),
        Some(version) => {
            let short = &version.sha256[..version.sha256.len().min(8)];
            format!("{} B (sha {short})", version.size)
        }
    }
}
