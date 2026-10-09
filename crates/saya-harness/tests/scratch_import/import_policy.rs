use super::*;

#[tokio::test]
async fn path_escapes_invalid_utf8_and_existing_tables_are_refused_or_replaced() {
    let root = std::env::temp_dir().join(format!("saya-import-policy-{}", std::process::id()));
    let outside = root.with_extension("outside.csv");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("ok.csv"), b"name\nnew\n").unwrap();
    std::fs::write(root.join("bad.csv"), b"name\n\xff\n").unwrap();
    std::fs::write(
        root.join("field.csv"),
        format!("name\n{}\n", "x".repeat(64 * 1024 + 1)),
    )
    .unwrap();
    let wide = format!(
        "{}\n{}\n",
        (0..513)
            .map(|i| format!("c{i}"))
            .collect::<Vec<_>>()
            .join(","),
        (0..513).map(|_| "x").collect::<Vec<_>>().join(",")
    );
    std::fs::write(root.join("wide.csv"), wide).unwrap();
    let huge = std::fs::File::create(root.join("huge.csv")).unwrap();
    huge.set_len((32 * 1024 * 1024 + 1) as u64).unwrap();
    std::fs::write(&outside, b"name\noutside\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, root.join("link.csv")).unwrap();
    let tool = ScratchSql::open(&root)
        .unwrap()
        .with_workspace(Arc::new(Workspace::open(&root).unwrap()));
    tool.execute(
        "scratch_sql",
        json!({"sql":"CREATE TABLE target (name VARCHAR)"}),
    )
    .await
    .unwrap();
    tool.execute(
        "scratch_sql",
        json!({"sql":"INSERT INTO target VALUES ('old')"}),
    )
    .await
    .unwrap();
    for arguments in [
        json!({"path":"ok.csv","table":"target","delimiter":1}),
        json!({"path":"ok.csv","table":"target","if_exists":null}),
    ] {
        assert!(tool.execute("scratch_import", arguments).await.is_err());
    }
    assert!(
        tool.execute(
            "scratch_import",
            json!({"path":"huge.csv","table":"target","if_exists":"replace"})
        )
        .await
        .is_err()
    );
    let unchanged = tool
        .execute("scratch_sql", json!({"sql":"SELECT name FROM target"}))
        .await
        .unwrap();
    assert_eq!(unchanged["rows"], json!([["old"]]));
    for path in ["../escape.csv", outside.to_str().unwrap(), "bad.csv"] {
        assert!(
            tool.execute(
                "scratch_import",
                json!({"path":path,"table":"target","if_exists":"replace"})
            )
            .await
            .is_err()
        );
    }
    #[cfg(unix)]
    assert!(
        tool.execute(
            "scratch_import",
            json!({"path":"link.csv","table":"target","if_exists":"replace"})
        )
        .await
        .is_err()
    );
    for path in ["wide.csv", "field.csv"] {
        assert!(
            tool.execute(
                "scratch_import",
                json!({"path":path,"table":"target","if_exists":"replace"})
            )
            .await
            .is_err()
        );
        let unchanged = tool
            .execute("scratch_sql", json!({"sql":"SELECT name FROM target"}))
            .await
            .unwrap();
        assert_eq!(
            unchanged["rows"],
            json!([["old"]]),
            "{path} must preserve target"
        );
    }
    assert!(
        tool.execute("scratch_import", json!({"path":"ok.csv","table":"target"}))
            .await
            .is_err()
    );
    tool.execute(
        "scratch_import",
        json!({"path":"ok.csv","table":"target","if_exists":"replace"}),
    )
    .await
    .unwrap();
    let rows = tool
        .execute("scratch_sql", json!({"sql":"SELECT name FROM target"}))
        .await
        .unwrap();
    assert_eq!(rows["rows"], json!([["new"]]));
    assert!(
        tool.execute(
            "scratch_sql",
            json!({"sql":format!("SELECT * FROM read_csv('{}')", root.join("ok.csv").display())})
        )
        .await
        .is_err()
    );
    let _ = (std::fs::remove_dir_all(root), std::fs::remove_file(outside));
}
