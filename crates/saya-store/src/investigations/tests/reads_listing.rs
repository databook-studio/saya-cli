use super::*;

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
