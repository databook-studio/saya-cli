use std::{fs, path::PathBuf, sync::Arc};

use saya_agent::ToolExecutor;
use saya_harness::workspace::Workspace;

use super::super::super::database_tools::DatabaseTools;
use super::{canonical_name, is_write_tool, prepare_coding_workspace_tools};

struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-ws-names-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws/src")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        ws.write("src/cli.mjs", b"export {}\n")
            .expect("contained write must succeed");
        ws.write("README.md", b"sandbox\n")
            .expect("a second root entry must exist so list(., 1) is not enough");
        Self { outer, ws }
    }

    fn tools(&self) -> DatabaseTools {
        DatabaseTools::with_registry(
            crate::connection::ConnectionRegistry::new("primary"),
            100,
            true,
            None,
        )
        .with_workspace(Some(Arc::new(self.ws.clone())))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

#[test]
fn short_names_are_canonical_and_old_workspace_names_stay_unknown() {
    assert_eq!(canonical_name("read"), "read");
    assert_eq!(canonical_name("list"), "list");
    assert_eq!(canonical_name("write"), "write");
    assert_eq!(canonical_name("edit"), "edit");
    assert_eq!(canonical_name("delete"), "delete");
    assert_eq!(canonical_name("bash"), "shell");
    for name in [
        "workspace_read",
        "workspace_list",
        "workspace_write",
        "workspace_edit",
        "workspace_delete",
    ] {
        assert_eq!(canonical_name(name), name, "{name} must not alias in");
        assert!(!is_write_tool(name), "{name} is not a write tool");
    }
    assert!(is_write_tool("write"));
    assert!(is_write_tool("edit"));
    assert!(is_write_tool("notebook_edit"));
    assert!(!is_write_tool("read"));
}

#[test]
fn workspace_definitions_advertise_the_short_names() {
    let tools = DatabaseTools::definitions(false, false, false, true);
    let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();
    for name in [
        "read",
        "list",
        "write",
        "edit",
        "notebook_edit",
        "glob",
        "grep",
    ] {
        assert!(names.contains(&name), "{name} must be advertised");
    }
    for name in [
        "workspace_read",
        "workspace_list",
        "workspace_write",
        "workspace_edit",
        "workspace_delete",
    ] {
        assert!(!names.contains(&name), "{name} must not be advertised");
    }
}

#[test]
fn coding_jobs_advertise_the_short_workspace_names_without_a_shell() {
    let mut tools = DatabaseTools::definitions(false, false, false, true);
    prepare_coding_workspace_tools(&mut tools);
    let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();
    for name in ["read", "list", "write", "edit", "glob", "grep"] {
        assert!(names.contains(&name), "{name} must be advertised");
    }
    // The root's `shell` is not added here: it is the sandboxed,
    // operator-approved `RootShell` the direct root composes itself.
    assert!(
        !names.contains(&"shell"),
        "the workspace layer adds no shell"
    );
}

#[tokio::test]
async fn an_old_workspace_name_is_an_unknown_tool() {
    let sandbox = Sandbox::new("old-name");
    let error = sandbox
        .tools()
        .execute(
            "workspace_write",
            serde_json::json!({"path": "src/cli.mjs", "content": "x"}),
        )
        .await
        .expect_err("an unadvertised name must not dispatch");
    assert_eq!(error, saya_agent::ToolError::UnsupportedTool);
}

#[tokio::test]
async fn read_of_a_directory_lists_its_entries() {
    let sandbox = Sandbox::new("read-dir");
    let tools = sandbox.tools();
    let file = tools
        .execute("read", serde_json::json!({"path": "src/cli.mjs"}))
        .await
        .expect("read of a file must succeed");
    assert_eq!(file["content"], "     1→export {}\n");
    let listed = tools
        .execute("read", serde_json::json!({"path": "src"}))
        .await
        .expect("read of a directory must list it");
    assert!(
        listed["entries"]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| entry["name"] == "cli.mjs")),
        "directory read must return entries: {listed}"
    );
}

// The workspace layer does not own `shell`: the direct root's sandboxed,
// operator-approved `RootShell` does, and `agent::tools::coding::root_shell_tests`
// exercises it (contained writes, `.git` denial, workdir resolution, approval).
