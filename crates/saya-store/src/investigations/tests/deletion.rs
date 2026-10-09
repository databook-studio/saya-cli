use super::*;

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
