use super::*;

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
