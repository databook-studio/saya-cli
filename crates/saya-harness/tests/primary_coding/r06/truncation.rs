use saya_harness::coding::context::{
    ChildBrief, ChildDependency, MAX_CONTEXT_BYTES, OriginalProjectContext,
};
use saya_store::{ProjectPreferenceStore, SqliteStateStore};
use saya_types::WorktreeId;

use super::support::{TempTree, authority, options};

async fn preferences(
    project: &TempTree,
    text: &str,
) -> (TempTree, Vec<saya_types::ProjectPreference>) {
    let workspace = project.workspace();
    let worktree = WorktreeId::from_canonical_path(workspace.root().to_str().unwrap()).unwrap();
    let database = TempTree::new();
    let store = SqliteStateStore::new(database.path().join("preferences.sqlite"));
    for index in 0..16 {
        store
            .add_project_preference(&worktree, text, index as i64 + 100)
            .await
            .unwrap();
    }
    let preferences = store.list_project_preferences(&worktree).await.unwrap();
    store.close().await;
    (database, preferences)
}

fn admitted_bytes(request: &saya_agent::AgentRequest) -> usize {
    request.prompt.len()
        + request
            .context_blocks
            .iter()
            .map(|block| block.label.len() + block.body.len())
            .sum::<usize>()
}

#[tokio::test(flavor = "current_thread")]
async fn optional_context_fills_exact_aggregate_cap_and_marks_truncation() {
    let project = TempTree::new();
    project.write("AGENTS.md", &vec![b'p'; 8 * 1024]);
    let personal = TempTree::new();
    personal.write("instructions.md", &vec![b'u'; 8 * 1024]);
    let (_database, preferences) = preferences(&project, &"v".repeat(1024)).await;
    let context =
        OriginalProjectContext::capture(&project.workspace(), Some(personal.path()), preferences)
            .unwrap();
    assert!(!context.instruction_sources()[0].source_truncated);

    let spec = context
        .project_child(
            authority("context-cap", "src/context.rs"),
            ChildBrief {
                task: "bounded task".to_owned(),
                dependencies: vec![ChildDependency {
                    id: "required-dependency".to_owned(),
                    summary: "optional dependency summary".to_owned(),
                }],
                artifacts: Vec::new(),
            },
            options("model"),
        )
        .unwrap();
    assert_eq!(admitted_bytes(spec.request()), MAX_CONTEXT_BYTES);
    assert!(
        spec.request()
            .context_blocks
            .iter()
            .any(|block| block.truncated)
    );
    assert!(spec.request().context_blocks.iter().any(|block| {
        block.label == "untrusted-dependency-identities"
            && block.body.contains("required-dependency")
    }));
    assert!(
        spec.request()
            .context_blocks
            .iter()
            .any(|block| { block.label == "untrusted-dependency-summaries" && block.truncated })
    );
}

#[tokio::test(flavor = "current_thread")]
async fn optional_context_truncation_preserves_utf8() {
    let project = TempTree::new();
    let unicode = "é".repeat(4 * 1024);
    project.write("AGENTS.md", unicode.as_bytes());
    let personal = TempTree::new();
    personal.write("instructions.md", unicode.as_bytes());
    let (_database, preferences) = preferences(&project, &"p".repeat(1024)).await;
    let context =
        OriginalProjectContext::capture(&project.workspace(), Some(personal.path()), preferences)
            .unwrap();

    let spec = context
        .project_child(
            authority("unicode-cap", "src/unicode.rs"),
            ChildBrief {
                task: "bounded task".to_owned(),
                dependencies: Vec::new(),
                artifacts: Vec::new(),
            },
            options("model"),
        )
        .unwrap();
    assert!(admitted_bytes(spec.request()) <= MAX_CONTEXT_BYTES);
    assert!(
        spec.request()
            .context_blocks
            .iter()
            .any(|block| block.truncated)
    );
    assert!(
        spec.request()
            .context_blocks
            .iter()
            .all(|block| std::str::from_utf8(block.body.as_bytes()).is_ok())
    );
}
