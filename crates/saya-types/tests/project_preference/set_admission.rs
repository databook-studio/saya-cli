//! Set-level admission: count, single-worktree, duplicate-id, and aggregate
//! recall bounds enforced by `validate_set`.

use super::*;

fn item(n: u32, text_len: usize) -> ProjectPreference {
    ProjectPreference {
        version: PROJECT_PREFERENCE_VERSION,
        id: pref_id(n),
        worktree: worktree(),
        text: "a".repeat(text_len),
        source: PreferenceSource::UserExplicit,
        created_unix_ms: CREATED,
        updated_unix_ms: CREATED,
    }
}

// ---------------------------------------------------------------------------
// Set-level admission
// ---------------------------------------------------------------------------

#[test]
fn a_set_of_64_is_admitted_and_65_refused() {
    let full: Vec<ProjectPreference> = (0..MAX_PROJECT_PREFERENCES as u32)
        .map(|n| item(n, 8))
        .collect();
    assert!(validate_set(&full).is_ok());
    let mut over = full;
    over.push(item(MAX_PROJECT_PREFERENCES as u32, 8));
    assert_eq!(
        validate_set(&over).unwrap_err(),
        PreferenceError::SetTooManyItems(65)
    );
}

#[test]
fn mixed_worktrees_are_refused() {
    let mut items = vec![item(0, 8), item(1, 8)];
    items[1].worktree = other_worktree();
    assert_eq!(
        validate_set(&items).unwrap_err(),
        PreferenceError::MixedWorktrees
    );
}

#[test]
fn duplicate_ids_are_refused() {
    let items = vec![item(0, 8), item(0, 8)];
    assert_eq!(
        validate_set(&items).unwrap_err(),
        PreferenceError::DuplicateId
    );
}

#[test]
fn aggregate_recall_is_capped_at_16_kib() {
    let full: Vec<ProjectPreference> = (0..16u32)
        .map(|n| item(n, MAX_PREFERENCE_TEXT_BYTES))
        .collect();
    assert_eq!(
        full.iter().map(|p| p.text.len()).sum::<usize>(),
        MAX_PREFERENCE_RECALL_BYTES
    );
    assert!(validate_set(&full).is_ok());
    let mut over = full;
    over.push(item(16, MAX_PREFERENCE_TEXT_BYTES));
    assert_eq!(
        validate_set(&over).unwrap_err(),
        PreferenceError::RecallOversize(17 * MAX_PREFERENCE_TEXT_BYTES)
    );
}
