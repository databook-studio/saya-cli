//! A read-only root (no working checkout) still reads its own folder: the
//! read-only workspace tools are advertised, the write family and the shell
//! are not, and a write call is refused.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use saya_agent::{ToolDefinition, ToolError, ToolExecutor};

use super::direct::RootTools;
use crate::agent::tools::child::tests_root_compose_fixture as fixture;

fn has(definitions: &[ToolDefinition], name: &str) -> bool {
    definitions.iter().any(|definition| definition.name == name)
}

/// The read-only surface never offers `shell`, so the data-file route that
/// names it cannot reach this job's prompt.
#[tokio::test]
async fn a_read_only_root_offers_no_shell() {
    let fixture = fixture::Fixture::new("read-only-shell", &[]).await;
    let root = RootTools::build(&fixture.delegate_job).await.unwrap().0;
    assert!(!root.offers_shell());
}

#[tokio::test]
async fn a_read_only_root_reads_its_folder_and_advertises_no_write() {
    let fixture = fixture::Fixture::new("read-only-root", &[]).await;
    let root = RootTools::build(&fixture.delegate_job).await.unwrap().0;
    let advertised = root.definitions();
    for name in ["read", "glob", "grep"] {
        assert!(
            has(&advertised, name),
            "{name} is advertised to a read-only job"
        );
    }
    for name in [
        "write",
        "edit",
        "delete",
        "notebook_edit",
        "shell",
        "helpers",
    ] {
        assert!(
            !has(&advertised, name),
            "{name} must not be advertised to a read-only job"
        );
    }

    let outcome = root
        .execute("read", serde_json::json!({"path": "tracked.txt"}))
        .await
        .expect("the read-only job reads its folder");
    let content = outcome["content"].as_str().unwrap_or_default();
    assert!(content.contains("original"), "{outcome}");

    let write = serde_json::json!({"path": "tracked.txt", "content": "changed\n"});
    assert!(
        matches!(
            root.execute("write", write).await,
            Err(ToolError::UnsupportedTool)
        ),
        "a write is refused as unknown"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.repository.root().join("tracked.txt")).unwrap(),
        "original\n",
        "the folder is unchanged"
    );
}
