//! Tests for the saved-investigation document repository (D2): bounded,
//! atomic, revision-checked JSON documents at `<root>/<id>.json` plus the
//! per-machine local binding at `<root>/local/<id>.json`.

use crate::investigations::{
    InvestigationListIssue, InvestigationRepository, InvestigationSummary, LocalBinding,
};
use crate::{StoreError, replace::FailingReplacer};
use saya_types::{
    MAX_SQL_BYTES, SqlDialect,
    investigation::{
        INVESTIGATION_FORMAT, INVESTIGATION_FORMAT_VERSION, InvestigationDefinitionV1,
        InvestigationId,
    },
};
use std::path::PathBuf;

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "saya-investigation-docs-{label}-{}",
        std::process::id()
    ))
}

fn id(name: &str) -> InvestigationId {
    InvestigationId::parse(name).unwrap()
}

fn definition(name_id: &str, revision: u32) -> InvestigationDefinitionV1 {
    InvestigationDefinitionV1 {
        format: INVESTIGATION_FORMAT.to_owned(),
        version: INVESTIGATION_FORMAT_VERSION,
        id: id(name_id),
        revision,
        name: "Demo investigation".to_owned(),
        description: None,
        sql: format!("select {revision}"),
        parameters: Vec::new(),
        dialect: SqlDialect::Sqlite,
        connection: "warehouse".to_owned(),
        objects: Vec::new(),
        schema_fingerprint: None,
        created_unix_ms: 1_000,
        updated_unix_ms: 1_000,
    }
}

fn document_bytes(name_id: &str, revision: u32) -> String {
    definition(name_id, revision).to_json_pretty().unwrap()
}

fn binding(name_id: &str) -> LocalBinding {
    LocalBinding {
        version: LocalBinding::VERSION,
        id: id(name_id),
        profile: "warehouse".to_owned(),
        profile_identity: "identity-1".to_owned(),
        reviewed_revision: 2,
        reviewed_schema_fingerprint: Some("schema-fp-1".to_owned()),
        reviewed_unix_ms: 2_000,
    }
}

fn summary_ids(page: &crate::investigations::InvestigationPage) -> Vec<&str> {
    page.summaries.iter().map(|s| s.id.as_str()).collect()
}

#[test]
fn create_then_get_roundtrips_definition() {
    let root = temp_root("roundtrip");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root.clone());
    let def = definition("roundtrip-me", 1);
    repo.create(&def).unwrap();
    assert_eq!(repo.get(&def.id).unwrap(), def);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn create_refuses_existing_id() {
    let root = temp_root("create-conflict");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root.clone());
    repo.create(&definition("twice", 1)).unwrap();
    assert_eq!(
        repo.create(&definition("twice", 1)),
        Err(StoreError::Conflict)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn create_refuses_at_collection_cap() {
    let root = temp_root("collection-cap");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let repo = InvestigationRepository::new(root.clone());
    let sample = document_bytes("cap-doc-000", 1);
    for index in 0..499 {
        let name = format!("cap-doc-{index:03}");
        std::fs::write(
            root.join(format!("{name}.json")),
            sample.replace("cap-doc-000", &name),
        )
        .unwrap();
    }
    repo.create(&definition("cap-doc-499", 1)).unwrap();
    assert_eq!(
        repo.create(&definition("cap-doc-500", 1)),
        Err(StoreError::LimitExceeded)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn create_refuses_oversize_serialization() {
    let root = temp_root("oversize");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root.clone());
    let mut def = definition("oversize-me", 1);
    def.sql = "x".repeat(MAX_SQL_BYTES);
    def.description = Some("d".repeat(2_048));
    def.objects = (0..256)
        .map(|index| format!("{}-{index:04}", "o".repeat(250)))
        .collect();
    assert_eq!(repo.create(&def), Err(StoreError::LimitExceeded));
    assert!(!root.join("oversize-me.json").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn create_validates_before_writing() {
    let root = temp_root("validate-first");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root.clone());
    let mut def = definition("invalid-me", 1);
    def.name = "   ".to_owned();
    assert_eq!(repo.create(&def), Err(StoreError::Invalid));
    assert!(!root.join("invalid-me.json").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn save_conflicting_revision_leaves_original_intact() {
    let root = temp_root("stale-update");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root.clone());
    repo.create(&definition("stale", 1)).unwrap();
    let original = std::fs::read(root.join("stale.json")).unwrap();

    assert_eq!(
        repo.update(&definition("stale", 2), 0),
        Err(StoreError::Conflict),
        "expected_revision behind the stored revision must be refused"
    );
    assert_eq!(
        repo.update(&definition("stale", 1), 1),
        Err(StoreError::Conflict),
        "a same-revision re-save must be refused"
    );
    assert_eq!(
        std::fs::read(root.join("stale.json")).unwrap(),
        original,
        "a refused update must leave the original bytes intact"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn update_advances_revision_and_persists() {
    let root = temp_root("update-ok");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root.clone());
    repo.create(&definition("advance", 1)).unwrap();
    repo.update(&definition("advance", 2), 1).unwrap();
    let loaded = repo.get(&id("advance")).unwrap();
    assert_eq!(loaded.revision, 2);
    assert_eq!(loaded.sql, "select 2");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn update_refuses_missing_document() {
    let root = temp_root("update-missing");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root);
    assert_eq!(
        repo.update(&definition("absent", 1), 0),
        Err(StoreError::NotFound)
    );
}

#[test]
fn update_refuses_document_with_mismatched_id() {
    let root = temp_root("mismatched-id");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("gamma.json"), document_bytes("delta", 1)).unwrap();
    let repo = InvestigationRepository::new(root.clone());
    assert_eq!(
        repo.update(&definition("gamma", 2), 1),
        Err(StoreError::Conflict),
        "a document whose internal id disagrees with its filename must not update"
    );
    assert_eq!(
        repo.get(&id("gamma")),
        Err(StoreError::Invalid),
        "get must report the inconsistent document, not heal it"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn interrupted_write_never_replaces_valid_definition() {
    let root = temp_root("interrupted");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root.clone());
    repo.create(&definition("interrupted", 1)).unwrap();
    let original = std::fs::read(root.join("interrupted.json")).unwrap();

    let error = repo
        .update_with_replacer(&definition("interrupted", 2), 1, &FailingReplacer)
        .unwrap_err();
    assert_eq!(error, StoreError::Unavailable);
    assert_eq!(
        std::fs::read(root.join("interrupted.json")).unwrap(),
        original,
        "a failed publish must leave the last good document byte-for-byte intact"
    );
    assert!(
        std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.path().extension().is_some_and(|ext| ext == "tmp")),
        "a failed publish must not leave staged temps behind"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn get_reports_corrupt_without_touching_file() {
    let root = temp_root("corrupt");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let repo = InvestigationRepository::new(root.clone());
    let path = root.join("corrupt-me.json");
    std::fs::write(&path, b"{ this is not json").unwrap();
    let original = std::fs::read(&path).unwrap();
    assert_eq!(repo.get(&id("corrupt-me")), Err(StoreError::Invalid));
    assert_eq!(
        std::fs::read(&path).unwrap(),
        original,
        "get must never touch a file it cannot parse"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn get_reports_unsupported_version() {
    let root = temp_root("future-version");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let repo = InvestigationRepository::new(root.clone());
    let bytes = document_bytes("future", 1).replace("\"version\": 1", "\"version\": 2");
    std::fs::write(root.join("future.json"), bytes).unwrap();
    assert_eq!(repo.get(&id("future")), Err(StoreError::VersionUnsupported));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn get_reports_oversize_document() {
    let root = temp_root("oversize-read");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let repo = InvestigationRepository::new(root.clone());
    let big = vec![b'x'; 128 * 1024 + 1];
    std::fs::write(root.join("huge.json"), &big).unwrap();
    assert_eq!(repo.get(&id("huge")), Err(StoreError::LimitExceeded));
}

#[test]
fn list_skips_temp_and_binding_files_and_reports_bad_files() {
    let root = temp_root("list-skip");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let repo = InvestigationRepository::new(root.clone());
    std::fs::write(root.join("alpha.json"), document_bytes("alpha", 1)).unwrap();
    std::fs::write(root.join("beta.json"), b"{ not json").unwrap();
    std::fs::write(root.join(".alpha.json.4242.0.tmp"), b"staged bytes").unwrap();
    std::fs::create_dir_all(root.join("local")).unwrap();
    std::fs::write(root.join("local/alpha.json"), b"binding bytes").unwrap();
    std::fs::write(root.join("README.json"), b"{}").unwrap();
    std::fs::create_dir_all(root.join("dirc.json")).unwrap();

    let page = repo.list(0, 50).unwrap();
    assert_eq!(page.total_seen, 2, "only id-stemmed .json files count");
    assert!(!page.capped);
    assert_eq!(
        page.summaries,
        vec![InvestigationSummary {
            id: id("alpha"),
            revision: 1,
            name: "Demo investigation".to_owned(),
            dialect: SqlDialect::Sqlite,
            connection: "warehouse".to_owned(),
            updated_unix_ms: 1_000,
        }]
    );
    assert_eq!(
        page.issues,
        vec![InvestigationListIssue {
            file_stem: "beta".to_owned(),
            error: "Invalid".to_owned(),
        }]
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn list_limit_bounds() {
    let root = temp_root("list-bounds");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root);
    assert_eq!(repo.list(0, 0), Err(StoreError::Invalid));
    assert_eq!(repo.list(0, 51), Err(StoreError::Invalid));
    let page = repo.list(0, 50).unwrap();
    assert_eq!(page.summaries.len(), 0);
    assert_eq!(page.issues.len(), 0);
    assert_eq!(page.total_seen, 0);
    assert!(!page.capped);
}

#[test]
fn list_pages_by_offset_and_reports_capped() {
    let root = temp_root("list-pages");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root.clone());
    for name in ["doc-a", "doc-b", "doc-c"] {
        repo.create(&definition(name, 1)).unwrap();
    }
    let page = repo.list(0, 2).unwrap();
    assert_eq!(summary_ids(&page), vec!["doc-a", "doc-b"]);
    assert!(page.capped, "more candidates exist beyond this page");
    assert_eq!(page.total_seen, 3);
    let page = repo.list(2, 2).unwrap();
    assert_eq!(summary_ids(&page), vec!["doc-c"]);
    assert!(!page.capped);
    let page = repo.list(5, 2).unwrap();
    assert_eq!(page.summaries.len(), 0);
    assert!(!page.capped);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn delete_removes_binding() {
    let root = temp_root("delete-binding");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root.clone());
    repo.create(&definition("gone", 1)).unwrap();
    repo.put_binding(&binding("gone")).unwrap();
    assert!(repo.get_binding(&id("gone")).unwrap().is_some());
    repo.delete(&id("gone"), 1).unwrap();
    assert_eq!(repo.get(&id("gone")), Err(StoreError::NotFound));
    assert_eq!(repo.get_binding(&id("gone")).unwrap(), None);
    assert!(!root.join("gone.json").exists());
    assert!(!root.join("local/gone.json").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn delete_clears_binding_before_document() {
    let root = temp_root("delete-order");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root.clone());
    repo.create(&definition("ordered", 1)).unwrap();

    // The binding clear is forced to fail: the binding path is a
    // directory, which remove_file cannot delete.
    std::fs::create_dir_all(root.join("local/ordered.json")).unwrap();
    assert_eq!(
        repo.delete(&id("ordered"), 1),
        Err(StoreError::Unavailable),
        "a failed binding clear must fail the delete"
    );
    assert!(
        root.join("ordered.json").exists(),
        "the binding is cleared before the document is removed, so a \
         failed clear must leave both in place, retryable"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn delete_refuses_wrong_revision_and_missing_document() {
    let root = temp_root("delete-refuse");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root.clone());
    repo.create(&definition("kept", 1)).unwrap();
    assert_eq!(repo.delete(&id("kept"), 9), Err(StoreError::Conflict));
    assert!(repo.get(&id("kept")).is_ok());
    assert_eq!(repo.delete(&id("missing"), 1), Err(StoreError::NotFound));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
#[cfg(unix)]
fn documents_are_written_0600_in_a_0700_root() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("modes-doc");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root.clone());
    repo.create(&definition("private", 1)).unwrap();
    let file_mode = std::fs::metadata(root.join("private.json"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(file_mode, 0o600);
    let root_mode = std::fs::metadata(&root).unwrap().permissions().mode() & 0o777;
    assert_eq!(root_mode, 0o700);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
#[cfg(unix)]
fn binding_roundtrip_and_permissions_0600() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("binding-roundtrip");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root.clone());
    let expected = binding("reviewed");
    repo.put_binding(&expected).unwrap();
    assert_eq!(repo.get_binding(&id("reviewed")).unwrap(), Some(expected));
    let file_mode = std::fs::metadata(root.join("local/reviewed.json"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(file_mode, 0o600);
    let dir_mode = std::fs::metadata(root.join("local"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dir_mode, 0o700);
    let root_mode = std::fs::metadata(&root).unwrap().permissions().mode() & 0o777;
    assert_eq!(root_mode, 0o700);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn binding_refuses_unknown_version() {
    let root = temp_root("binding-version");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("local")).unwrap();
    let repo = InvestigationRepository::new(root.clone());
    let mut binding = binding("versioned");
    binding.version = 2;
    std::fs::write(
        root.join("local/versioned.json"),
        serde_json::to_string_pretty(&binding).unwrap(),
    )
    .unwrap();
    assert_eq!(
        repo.get_binding(&id("versioned")),
        Err(StoreError::VersionUnsupported)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn binding_refuses_mismatched_internal_id() {
    let root = temp_root("binding-id");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("local")).unwrap();
    let repo = InvestigationRepository::new(root.clone());
    std::fs::write(
        // binding("delta") stored under gamma's name
        root.join("local/gamma.json"),
        serde_json::to_string_pretty(&binding("delta")).unwrap(),
    )
    .unwrap();
    assert_eq!(repo.get_binding(&id("gamma")), Err(StoreError::Invalid));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn binding_oversize_refused() {
    let root = temp_root("binding-oversize");
    let _ = std::fs::remove_dir_all(&root);
    let repo = InvestigationRepository::new(root.clone());
    let mut binding = binding("big");
    binding.profile = "p".repeat(17 * 1024);
    assert_eq!(repo.put_binding(&binding), Err(StoreError::LimitExceeded));
    assert_eq!(repo.get_binding(&id("big")).unwrap(), None);
    let _ = std::fs::remove_dir_all(root);
}
