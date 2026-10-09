/// A symlinked path is refused, never followed: the victim outside the
/// workspace keeps its bytes and the link itself is not replaced by a file.
/// (Unix-only: the symlink plant needs the unix fs API.)
#[cfg(unix)]
#[tokio::test]
async fn workspace_write_refuses_a_symlinked_path() {
    let sandbox = Sandbox::new("symlink");
    fs::write(sandbox.outer.join("victim.txt"), b"victim").expect("victim plant must succeed");
    std::os::unix::fs::symlink(
        sandbox.outer.join("victim.txt"),
        sandbox.ws_root().join("link"),
    )
    .expect("symlink plant must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "link", "content": "overwritten?"}),
        )
        .await
        .expect_err("a symlinked path must be refused, not followed");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("symlink"),
                "the refusal must name the symlink refusal: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.outer.join("victim.txt")).expect("victim must survive"),
        b"victim",
        "the link must not have been followed"
    );
    assert!(
        sandbox
            .ws_root()
            .join("link")
            .symlink_metadata()
            .expect("link must survive")
            .file_type()
            .is_symlink(),
        "the link must not have been replaced by a regular file"
    );
}

/// Content over the byte bound is a typed refusal carrying the bound — never
/// a truncated write — and leaves no file behind. Exactly at the bound is
/// allowed, so the bound is a ceiling, not a hint.
#[tokio::test]
async fn workspace_write_refuses_content_over_the_bound_and_leaves_no_file() {
    let sandbox = Sandbox::new("bound");
    let tools = sandbox.tools();
    let oversized = "a".repeat(WORKSPACE_WRITE_MAX_BYTES + 1);
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "big.txt", "content": oversized}),
        )
        .await
        .expect_err("over the bound must be refused whole");
    assert_eq!(
        error,
        ToolError::WorkspaceWriteTooLarge {
            limit: WORKSPACE_WRITE_MAX_BYTES
        }
    );
    assert!(
        !sandbox.ws_root().join("big.txt").exists(),
        "a refused write must leave no file behind"
    );
    // The boundary case: exactly the bound is a legal write.
    let exact = "a".repeat(WORKSPACE_WRITE_MAX_BYTES);
    tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "exact.txt", "content": exact}),
        )
        .await
        .expect("content exactly at the bound must be accepted");
    assert_eq!(
        fs::metadata(sandbox.ws_root().join("exact.txt"))
            .expect("file must exist")
            .len(),
        WORKSPACE_WRITE_MAX_BYTES as u64
    );
}

/// Writing over an existing file with the digest the model observed replaces
/// it whole: the path never holds a mix of old and new content, and no temp
/// file survives the replace. (Premise updated by E03: replacement now
/// requires `expected_digest`, so the second call carries the digest from a
/// fresh read.)
#[tokio::test]
async fn workspace_write_replaces_an_existing_file_whole() {
    let sandbox = Sandbox::new("replace");
    let tools = sandbox.tools();
    tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "report.md", "content": "0123456789ABCDEF"}),
        )
        .await
        .expect("the first write must succeed");
    let observed = tools
        .execute("workspace_read", serde_json::json!({"path": "report.md"}))
        .await
        .expect("the file must read back");
    tools
        .execute(
            "workspace_write",
            serde_json::json!({
                "path": "report.md",
                "content": "hi",
                "expected_digest": observed["digest"],
            }),
        )
        .await
        .expect("a permitted digest-guarded replace must succeed");
    assert_eq!(
        fs::read(sandbox.ws_root().join("report.md")).expect("file must exist"),
        b"hi",
        "the path must hold exactly the new content — no partial window's debris"
    );
    let leftovers: Vec<String> = fs::read_dir(sandbox.ws_root())
        .expect("root must exist")
        .map(|entry| {
            entry
                .expect("entries must read")
                .file_name()
                .into_string()
                .expect("utf-8")
        })
        .collect();
    assert_eq!(leftovers, vec!["report.md"], "no temp file may survive");
}
