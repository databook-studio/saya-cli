//! Tests for the preference projection: a worktree's explicit preferences
//! reach the request as one bounded, untrusted, labelled context block on the
//! user turn — after the instruction blocks, never in the system prompt,
//! never from another worktree, independent of the database privacy gate.

use super::{project_preferences, render_preferences};
use crate::agent::context::sources::UNTRUSTED_PREFIX;
use crate::commands::preferences::worktree_id_for;
use saya_agent::render_untrusted_block;
use saya_store::{ProjectPreferenceStore, SqliteStateStore};
use saya_types::{
    PROJECT_PREFERENCE_VERSION, PreferenceId, PreferenceSource, ProjectPreference, WorktreeId,
};
use std::path::{Path, PathBuf};

const CLOSE: &str = "<<<CONTEXT_BLOCK_END>>>";
const OPEN: &str = "<<<CONTEXT_BLOCK_BEGIN>>>";
const ESCAPED_CLOSE: &str = "<<<\\CONTEXT_BLOCK_END>>>";
const HEADER: &str =
    "Explicit project preferences confirmed by the user for this worktree (oldest first):";

fn stamp(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "saya-pref-ctx-{tag}-{}-{nanos}",
        std::process::id()
    ))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// A hand-built preference, for the pure renderer: the store refuses
/// secret-shaped text at add time, so the redaction pass is exercised here.
fn item(text: &str, created: i64, tag: usize) -> ProjectPreference {
    ProjectPreference {
        version: PROJECT_PREFERENCE_VERSION,
        id: PreferenceId::parse(&format!("pp-{tag:032x}")).expect("the id matches the pp- shape"),
        worktree: WorktreeId::from_canonical_path("/saya-prefs-test").expect("the worktree shape"),
        text: text.to_owned(),
        source: PreferenceSource::UserExplicit,
        created_unix_ms: created,
        updated_unix_ms: created,
    }
}

async fn seed(store: &SqliteStateStore, worktree: &WorktreeId, text: &str, now: i64) {
    store
        .add_project_preference(worktree, text, now)
        .await
        .expect("the store accepts the seeded text");
}

/// A unit-test fixture: the stamp root, created, plus its canonical path —
/// the exact string the worktree identity hashes.
fn canonical_fixture(tag: &str) -> (PathBuf, PathBuf) {
    let root = stamp(tag);
    std::fs::create_dir_all(&root).expect("create the fixture root");
    let canonical = std::fs::canonicalize(&root).expect("the fixture dir canonicalises");
    (root, canonical)
}

include!("preferences_tests/absence_and_format.rs");
