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

fn with_text(text: String) -> ProjectPreference {
    ProjectPreference { text, ..record() }
}

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

fn wire_with(id: &str, wt: &str, text: &str) -> String {
    format!(
        r#"{{"version":1,"id":"{id}","worktree":"{wt}","text":"{text}","source":"user_explicit","created_unix_ms":1700000000000,"updated_unix_ms":1700000000000}}"#
    )
}

fn wire_extra(extra: &str) -> String {
    let id = pref_id(0);
    let wt = worktree();
    format!(
        r#"{{"version":1,"id":"{id}","worktree":"{wt}","text":"t","source":"user_explicit","created_unix_ms":1700000000000,"updated_unix_ms":1700000000000{extra}}}"#,
        id = id.as_str(),
        wt = wt.as_str(),
    )
}

fn wire_source(source: &str) -> String {
    let id = pref_id(0);
    let wt = worktree();
    format!(
        r#"{{"version":1,"id":"{id}","worktree":"{wt}","text":"t","source":"{source}","created_unix_ms":1700000000000,"updated_unix_ms":1700000000000}}"#,
        id = id.as_str(),
        wt = wt.as_str(),
    )
}

fn wire_version(version: u32) -> String {
    let id = pref_id(0);
    let wt = worktree();
    format!(
        r#"{{"version":{version},"id":"{id}","worktree":"{wt}","text":"t","source":"user_explicit","created_unix_ms":1700000000000,"updated_unix_ms":1700000000000}}"#,
        id = id.as_str(),
        wt = wt.as_str(),
    )
}

// ---------------------------------------------------------------------------
// The bounds pin their scale, so a loosened bound is visible in the diff
// ---------------------------------------------------------------------------

#[test]
fn the_bounds_pin_their_scale() {
    assert_eq!(PROJECT_PREFERENCE_VERSION, 1);
    assert_eq!(MAX_PROJECT_PREFERENCES, 64);
    assert_eq!(MAX_PREFERENCE_TEXT_BYTES, 1024);
    assert_eq!(MAX_PREFERENCE_RECALL_BYTES, 16 * 1024);
    assert_eq!(MAX_PREFERENCE_RECORD_BYTES, 4 * 1024);
}

// ---------------------------------------------------------------------------
// Valid records round-trip; unknown versions and unknown fields are refused
// ---------------------------------------------------------------------------

#[test]
fn a_valid_record_round_trips() {
    let record = record();
    let bytes = serde_json::to_vec(&record).unwrap();
    assert_eq!(ProjectPreference::from_json_bytes(&bytes).unwrap(), record);
}

#[test]
fn unsupported_versions_are_refused() {
    for version in [0u32, 2] {
        assert_eq!(
            ProjectPreference::from_json_bytes(wire_version(version).as_bytes()).unwrap_err(),
            PreferenceError::UnsupportedVersion(version)
        );
    }
}

#[test]
fn an_unknown_field_is_refused() {
    for extra in [r#","command":"rm -rf /""#, r#","grants":["sql:execute"]"#] {
        assert_eq!(
            ProjectPreference::from_json_bytes(wire_extra(extra).as_bytes()).unwrap_err(),
            PreferenceError::Malformed
        );
    }
}

#[test]
fn assistant_inferred_sources_are_unrepresentable() {
    for source in ["assistant_inferred", "generated"] {
        assert_eq!(
            ProjectPreference::from_json_bytes(wire_source(source).as_bytes()).unwrap_err(),
            PreferenceError::Malformed
        );
    }
}

// ---------------------------------------------------------------------------
// Text bounds, counted in bytes
// ---------------------------------------------------------------------------

#[test]
fn text_bounds_are_enforced_in_bytes() {
    assert!(
        with_text("a".repeat(MAX_PREFERENCE_TEXT_BYTES))
            .validate()
            .is_ok()
    );
    assert_eq!(
        with_text("a".repeat(MAX_PREFERENCE_TEXT_BYTES + 1))
            .validate()
            .unwrap_err(),
        PreferenceError::TextOversize(MAX_PREFERENCE_TEXT_BYTES + 1)
    );
    // Multi-byte text is counted in bytes, not chars: 512 `é` is 1024 bytes.
    assert!(with_text("é".repeat(512)).validate().is_ok());
    assert!(with_text("é".repeat(513)).validate().is_err());
}

#[test]
fn empty_and_whitespace_text_is_refused() {
    for text in ["", " ", "\t \n"] {
        assert_eq!(
            with_text(text.to_string()).validate().unwrap_err(),
            PreferenceError::TextEmpty
        );
    }
}

#[test]
fn control_characters_are_refused_but_newline_and_tab_are_allowed() {
    assert!(
        with_text("line one\nline two\tindented".to_string())
            .validate()
            .is_ok()
    );
    for text in ["a\u{0}b", "a\u{1}b", "a\rb", "a\u{7}b"] {
        assert_eq!(
            with_text(text.to_string()).validate().unwrap_err(),
            PreferenceError::TextControlCharacter
        );
    }
}

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

// ---------------------------------------------------------------------------
// Edits keep identity; only text and updated change
// ---------------------------------------------------------------------------

#[test]
fn edited_keeps_identity_and_bumps_updated() {
    let base = record();
    let edited = base.edited("prefer compact tables", CREATED + 5).unwrap();
    assert_eq!(edited.id, base.id);
    assert_eq!(edited.worktree, base.worktree);
    assert_eq!(edited.source, PreferenceSource::UserExplicit);
    assert_eq!(edited.created_unix_ms, CREATED);
    assert_eq!(edited.updated_unix_ms, CREATED + 5);
    assert_eq!(edited.text, "prefer compact tables");
    // `now` before `created` is clamped, never reordered.
    let clamped = base.edited("prefer compact tables", CREATED - 100).unwrap();
    assert_eq!(clamped.updated_unix_ms, CREATED);
}

#[test]
fn edited_refuses_invalid_text_and_never_rebinds_the_worktree() {
    let base = record();
    assert_eq!(
        base.edited("", CREATED + 1).unwrap_err(),
        PreferenceError::TextEmpty
    );
    assert_eq!(
        base.edited(&"a".repeat(MAX_PREFERENCE_TEXT_BYTES + 1), CREATED + 1)
            .unwrap_err(),
        PreferenceError::TextOversize(MAX_PREFERENCE_TEXT_BYTES + 1)
    );
    // The edit takes no worktree parameter, so rebinding is unrepresentable;
    // the result carries the original worktree however the edit is made.
    assert_ne!(other_worktree(), base.worktree);
    let edited = base.edited("same worktree only", CREATED + 1).unwrap();
    assert_eq!(edited.worktree, base.worktree);
}

// ---------------------------------------------------------------------------
// Timestamps
// ---------------------------------------------------------------------------

#[test]
fn timestamps_must_be_ordered_and_non_negative() {
    let mut late = record();
    late.updated_unix_ms = CREATED - 1;
    assert_eq!(
        late.validate().unwrap_err(),
        PreferenceError::TimestampOrder
    );
    let mut negative = record();
    negative.created_unix_ms = -1;
    negative.updated_unix_ms = -1;
    assert_eq!(
        negative.validate().unwrap_err(),
        PreferenceError::NegativeTimestamp
    );
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

// ---------------------------------------------------------------------------
// The record byte cap fires before any parsing
// ---------------------------------------------------------------------------

#[test]
fn wire_records_over_the_record_cap_are_refused_before_parsing() {
    let text = "a".repeat(MAX_PREFERENCE_RECORD_BYTES + 1);
    let json = wire_with(pref_id(0).as_str(), worktree().as_str(), &text);
    assert_eq!(
        ProjectPreference::from_json_bytes(json.as_bytes()).unwrap_err(),
        PreferenceError::Oversize(json.len())
    );
}
