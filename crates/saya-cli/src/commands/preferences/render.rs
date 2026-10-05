//! Rendering of typed preference outcomes (M04a): one function from
//! [`PreferenceOutcome`] to `(exit code, message)`, so the CLI subcommand
//! and the slash packet (M04b) cannot disagree about what an outcome says.
//! Success exits 0; every refusal exits 2 with a one-sentence remedy. The
//! retention sentence rides `list` and `add` verbatim.

use super::{PreferenceOutcome, PreferenceRefusal};
use crate::interactive::session_continuation::now_unix_ms;
use saya_types::{MAX_PROJECT_PREFERENCES, ProjectPreference};

/// The retention sentence, stated on `list` and `add` outputs.
pub(crate) const RETENTION_NOTE: &str = "Project preferences are explicit guidance you confirmed for this worktree. They never grant permissions and are stored separately from database knowledge (`saya contracts`).";

/// Every refusal or failure of `saya preferences` is the documented usage
/// class; success is 0.
pub(crate) const EXIT_PREFERENCES_ERROR: i32 = 2;

/// Renders one outcome: the exit code (0 success, 2 refusal) and the
/// message. Never prompts, never reads stdin.
pub(crate) fn render(outcome: &PreferenceOutcome) -> (i32, String) {
    match outcome {
        PreferenceOutcome::Added(record) => (
            0,
            format!("Added preference {}.\n{RETENTION_NOTE}", record.id.as_str()),
        ),
        PreferenceOutcome::Listed {
            worktree_label,
            items,
        } => (0, render_list(worktree_label, items)),
        PreferenceOutcome::Edited(record) => (
            0,
            format!(
                "Edited preference {}.\n{RETENTION_NOTE}",
                record.id.as_str()
            ),
        ),
        PreferenceOutcome::Forgotten { existed, id } => {
            if *existed {
                (0, format!("Forgot preference {id}."))
            } else {
                // Decided (M04a): a missing id is a refusal, consistent with
                // `sessions delete` and `investigation delete`.
                (
                    EXIT_PREFERENCES_ERROR,
                    format!(
                        "Nothing was removed: no preference {id} in this worktree. \
                         Run `saya preferences list` to see the current ids."
                    ),
                )
            }
        }
        PreferenceOutcome::ForgottenAll(removed) => (0, format!("Forgot {removed} preferences.")),
        PreferenceOutcome::Refused(refusal) => (EXIT_PREFERENCES_ERROR, refusal_message(*refusal)),
    }
}

/// The listing: a count header naming the worktree and the cap, one line
/// per preference (`<id>  <age>  <one-line text>`), then the retention
/// sentence. An empty listing states so and names the add form.
fn render_list(worktree_label: &str, items: &[ProjectPreference]) -> String {
    if items.is_empty() {
        let q = '"';
        return format!(
            "No project preferences for {worktree_label}. Add one with: \
             saya preferences add {q}<text>{q}"
        );
    }
    let mut lines = vec![format!(
        "Project preferences for {worktree_label} ({} of {MAX_PROJECT_PREFERENCES}):",
        items.len()
    )];
    for item in items {
        lines.push(format!(
            "{}  {}  {}",
            item.id.as_str(),
            age_label(item.created_unix_ms),
            one_line(&item.text)
        ));
    }
    lines.push(RETENTION_NOTE.to_owned());
    lines.join("\n")
}

/// A stored preference lists on one line: newlines become ` ⏎ `, tabs a
/// space — a multi-line text can never break the line-per-item shape.
fn one_line(text: &str) -> String {
    text.replace(char::from(10), " ⏎ ")
        .replace(char::from(9), " ")
}

/// The item's age, or its created date once it is older than a day.
fn age_label(created_unix_ms: i64) -> String {
    let age_seconds = (now_unix_ms() - created_unix_ms).max(0) / 1000;
    if age_seconds < 60 {
        format!("{age_seconds}s ago")
    } else if age_seconds < 3600 {
        format!("{}m ago", age_seconds / 60)
    } else if age_seconds < 86_400 {
        format!("{}h ago", age_seconds / 3600)
    } else {
        chrono::DateTime::from_timestamp_millis(created_unix_ms).map_or_else(
            || format!("{created_unix_ms}ms"),
            |stamp| stamp.format("%Y-%m-%d").to_string(),
        )
    }
}

/// Every refusal states one remedy.
fn refusal_message(refusal: PreferenceRefusal) -> String {
    match refusal {
        PreferenceRefusal::NoWorktree => {
            "no project worktree here: run this inside a git worktree (preferences are scoped to one)"
                .to_owned()
        }
        PreferenceRefusal::TextNotAccepted => {
            "not accepted: a preference must be 1–1024 bytes of plain text without control \
             characters, and must not look like a secret (keys, tokens, passwords, \
             credentials in URLs)"
                .to_owned()
        }
        PreferenceRefusal::TooMany => {
            "this worktree already holds the 64-preference limit: forget one with \
             `saya preferences forget <id>` (or all with `forget-all`) and add again"
                .to_owned()
        }
        PreferenceRefusal::TooMuchText => {
            "this worktree's preferences are at the 16 KiB aggregate cap: shorten the text \
             or forget some preferences first"
                .to_owned()
        }
        PreferenceRefusal::NotFound => {
            "no preference with that id in this worktree: run `saya preferences list` \
             and copy an id from it"
                .to_owned()
        }
        PreferenceRefusal::InvalidId => {
            "that is not a preference id (they look like `pp-` followed by 32 hex characters): \
             copy an id from `saya preferences list`"
                .to_owned()
        }
        PreferenceRefusal::Unavailable => {
            "the preferences store is unavailable: check that the state database path is \
             writable and try again"
                .to_owned()
        }
    }
}
