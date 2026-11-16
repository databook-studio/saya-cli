use super::*;

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
