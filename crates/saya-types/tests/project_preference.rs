//! Contract tests for explicit worktree preference records in `saya_types`.
//!
//! A project preference is short, user-confirmed guidance text scoped to one
//! canonical worktree, read back from storage as untrusted input, so every
//! gate re-runs on the way in: the byte cap, the lenient version probe, the
//! strict parse, and `validate()`. Only explicit user provenance exists,
//! nothing in the record can carry authority, and an edit can never rebind
//! another worktree.

use saya_types::{
    MAX_PREFERENCE_RECALL_BYTES, MAX_PREFERENCE_RECORD_BYTES, MAX_PREFERENCE_TEXT_BYTES,
    MAX_PROJECT_PREFERENCES, PROJECT_PREFERENCE_VERSION, PreferenceError, PreferenceId,
    PreferenceSource, ProjectPreference, WorktreeId, validate_set,
};

const CREATED: i64 = 1_700_000_000_000;

fn worktree() -> WorktreeId {
    WorktreeId::from_canonical_path("/work/saya").unwrap()
}

fn other_worktree() -> WorktreeId {
    WorktreeId::from_canonical_path("/work/other").unwrap()
}

fn pref_id(n: u32) -> PreferenceId {
    PreferenceId::parse(&format!("pp-{n:032x}")).unwrap()
}

fn record() -> ProjectPreference {
    ProjectPreference {
        version: PROJECT_PREFERENCE_VERSION,
        id: pref_id(0),
        worktree: worktree(),
        text: "prefer Europe/London for date grouping".to_string(),
        source: PreferenceSource::UserExplicit,
        created_unix_ms: CREATED,
        updated_unix_ms: CREATED,
    }
}

fn wire_with(id: &str, wt: &str, text: &str) -> String {
    format!(
        r#"{{"version":1,"id":"{id}","worktree":"{wt}","text":"{text}","source":"user_explicit","created_unix_ms":1700000000000,"updated_unix_ms":1700000000000}}"#
    )
}

#[path = "project_preference/editing.rs"]
mod editing;
#[path = "project_preference/identity_shapes.rs"]
mod identity_shapes;
#[path = "project_preference/set_admission.rs"]
mod set_admission;
#[path = "project_preference/text_validation.rs"]
mod text_validation;
#[path = "project_preference/wire_acceptance.rs"]
mod wire_acceptance;
