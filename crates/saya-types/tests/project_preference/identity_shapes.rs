//! Identity shapes, on the wire and in code: `PreferenceId` and
//! `WorktreeId` parse rules, `from_canonical_path` determinism, and the
//! wire-level refusals `from_json_bytes` raises for bad shapes.

use super::*;

// ---------------------------------------------------------------------------
// Identity shapes, on the wire and in code
// ---------------------------------------------------------------------------

#[test]
fn wire_records_with_bad_ids_are_refused_by_validate() {
    assert_eq!(
        ProjectPreference::from_json_bytes(wire_with("nope", worktree().as_str(), "t").as_bytes())
            .unwrap_err(),
        PreferenceError::InvalidId
    );
}

#[test]
fn wire_records_with_bad_worktree_shapes_are_refused_by_validate() {
    assert_eq!(
        ProjectPreference::from_json_bytes(wire_with(pref_id(0).as_str(), "nope", "t").as_bytes())
            .unwrap_err(),
        PreferenceError::InvalidWorktree
    );
}

#[test]
fn id_and_worktree_parse_reject_bad_shapes() {
    assert!(PreferenceId::parse("pp-0011").is_err());
    assert!(PreferenceId::parse("pp-00112233445566778899aabbccddeefF").is_err());
    assert!(PreferenceId::parse("ck-00112233445566778899aabbccddeeff").is_err());
    assert!(WorktreeId::parse("wt-0011").is_err());
    assert!(
        WorktreeId::parse("p-00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff")
            .is_err()
    );
}

#[test]
fn from_canonical_path_is_deterministic_and_distinguishes_paths() {
    let a = WorktreeId::from_canonical_path("/work/saya").unwrap();
    let b = WorktreeId::from_canonical_path("/work/saya").unwrap();
    assert_eq!(a, b);
    assert_ne!(a, other_worktree());
    assert_eq!(a.as_str().len(), 3 + 64);
    assert!(WorktreeId::parse(a.as_str()).is_ok());
}

#[test]
fn from_canonical_path_refuses_empty_and_nul_paths() {
    assert_eq!(
        WorktreeId::from_canonical_path("").unwrap_err(),
        PreferenceError::InvalidCanonicalPath
    );
    assert_eq!(
        WorktreeId::from_canonical_path("a\u{0}b").unwrap_err(),
        PreferenceError::InvalidCanonicalPath
    );
}
