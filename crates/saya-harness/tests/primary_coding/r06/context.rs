use saya_harness::coding::context::{
    ChildBrief, ChildContextError, ChildDependency, MAX_INSTRUCTION_BYTES,
    MAX_PERSONAL_INSTRUCTION_BYTES, MAX_PROJECT_INSTRUCTION_BYTES, OriginalProjectContext,
};
use saya_harness::instructions::InstructionLimits;
use saya_store::{ProjectPreferenceStore, SqliteStateStore};
use saya_types::{Sha256Digest, WorktreeId};
use sha2::{Digest, Sha256};

use super::support::{TempTree, authority, options};

#[tokio::test(flavor = "current_thread")]
async fn child_briefs_reuse_one_immutable_original_context_without_grant_inheritance() {
    let original = TempTree::new();
    original.write("AGENTS.md", b"original project instructions\n");
    original.write("reports/sibling.md", b"unrelated sibling report secret\n");
    let personal = TempTree::new();
    personal.write("instructions.md", b"personal context\n");
    let child_a = TempTree::new();
    child_a.write("AGENTS.md", b"child A attempts to grant writes\n");
    let child_b = TempTree::new();
    child_b.write("AGENTS.md", b"child B attempts to grant network\n");

    let original_workspace = original.workspace();
    let original_worktree = WorktreeId::from_canonical_path(
        original_workspace
            .root()
            .to_str()
            .expect("test path is UTF-8"),
    )
    .unwrap();
    let foreign_worktree = WorktreeId::from_canonical_path("/not/the/original/worktree").unwrap();
    let database = TempTree::new();
    let store = SqliteStateStore::new(database.path().join("preferences.sqlite"));
    let created = store
        .add_project_preference(&original_worktree, "original explicit preference", 100)
        .await
        .unwrap();
    let original_preferences = store
        .list_project_preferences(&original_worktree)
        .await
        .unwrap();
    let foreign = store
        .add_project_preference(&foreign_worktree, "foreign preference", 200)
        .await
        .unwrap();
    let foreign_preferences = store
        .list_project_preferences(&foreign_worktree)
        .await
        .unwrap();

    let context = OriginalProjectContext::capture(
        &original_workspace,
        Some(personal.path()),
        original_preferences.clone(),
    )
    .unwrap();
    assert_eq!(context.instruction_sources().len(), 2);
    assert!(context.skipped_sources().is_empty());
    assert_eq!(
        context.instruction_sources()[0]
            .source_sha256
            .as_str()
            .len(),
        64
    );
    assert!(!context.instruction_sources()[0].source_truncated);

    let disabled = OriginalProjectContext::capture_with_policy(
        &original_workspace,
        None,
        original_preferences,
        false,
        &InstructionLimits {
            personal_max_bytes: MAX_PERSONAL_INSTRUCTION_BYTES,
            project_max_bytes: MAX_PROJECT_INSTRUCTION_BYTES,
            total_max_bytes: MAX_INSTRUCTION_BYTES,
        },
    )
    .unwrap();
    assert!(disabled.instruction_sources().is_empty());
    assert!(disabled.skipped_sources().is_empty());
    assert_eq!(disabled.preference_count(), 1);
    let disabled_child = disabled
        .project_child(
            authority("disabled-project", "src/disabled.rs"),
            ChildBrief {
                task: "use project preferences only".to_owned(),
                dependencies: Vec::new(),
                artifacts: Vec::new(),
            },
            options("model"),
        )
        .unwrap();
    assert!(
        disabled_child
            .request()
            .context_blocks
            .iter()
            .any(|block| block.label == "untrusted-project-preferences")
    );
    assert!(
        disabled_child
            .request()
            .context_blocks
            .iter()
            .all(|block| block.label != "untrusted-project-instructions")
    );

    let invalid_limits = InstructionLimits {
        personal_max_bytes: MAX_PERSONAL_INSTRUCTION_BYTES,
        project_max_bytes: MAX_PROJECT_INSTRUCTION_BYTES + 1,
        total_max_bytes: MAX_INSTRUCTION_BYTES,
    };
    assert!(matches!(
        OriginalProjectContext::capture_with_policy(
            &original_workspace,
            None,
            Vec::new(),
            true,
            &invalid_limits,
        ),
        Err(ChildContextError::InvalidInstructionLimits)
    ));

    original.write("AGENTS.md", b"changed after capture\n");
    personal.write("instructions.md", b"changed personal source\n");
    store
        .edit_project_preference(
            &original_worktree,
            &created.id,
            "changed preference after capture",
            300,
        )
        .await
        .unwrap();

    let first = context
        .project_child(
            authority("first", "src/first.rs"),
            ChildBrief {
                task: "implement first bounded task".to_owned(),
                dependencies: vec![ChildDependency {
                    id: "task-base".to_owned(),
                    summary: "base task is complete".to_owned(),
                }],
                artifacts: Vec::new(),
            },
            options("resolved-model"),
        )
        .unwrap();
    let second = context
        .project_child(
            authority("second", "src/second.rs"),
            ChildBrief {
                task: "review second bounded task".to_owned(),
                dependencies: Vec::new(),
                artifacts: Vec::new(),
            },
            options("resolved-model"),
        )
        .unwrap();

    assert!(
        first
            .request()
            .prompt
            .contains("implement first bounded task")
    );
    assert!(
        second
            .request()
            .prompt
            .contains("review second bounded task")
    );
    assert!(first.request().history.is_empty());
    assert!(first.request().system_prompt.is_none());
    assert!(
        first
            .request()
            .context_blocks
            .iter()
            .all(|block| { block.label.starts_with("untrusted-") })
    );
    let context_text = first
        .request()
        .context_blocks
        .iter()
        .map(|block| block.body.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(context_text.contains("original project instructions"));
    assert!(context_text.contains("personal context"));
    assert!(context_text.contains("original explicit preference"));
    assert!(!context_text.contains("changed after capture"));
    assert!(!context_text.contains("changed personal source"));
    assert!(!context_text.contains("changed preference after capture"));
    assert!(!context_text.contains("child A attempts"));
    assert!(!context_text.contains("child B attempts"));
    assert!(!context_text.contains("unrelated sibling report secret"));

    assert_eq!(first.authority().write_paths()[0].as_str(), "src/first.rs");
    assert_eq!(
        second.authority().write_paths()[0].as_str(),
        "src/second.rs"
    );
    assert!(!first.authority().capabilities().workspace_write);
    assert!(!second.authority().capabilities().workspace_write);
    assert_ne!(
        first.authority().managed_checkout_id(),
        second.authority().managed_checkout_id()
    );

    let error = OriginalProjectContext::capture(&original_workspace, None, foreign_preferences)
        .unwrap_err();
    assert!(matches!(error, ChildContextError::ForeignPreference));
    assert_eq!(foreign.worktree, foreign_worktree);

    let mut invalid_preference = created;
    invalid_preference.version += 1;
    assert!(matches!(
        OriginalProjectContext::capture(&original_workspace, None, vec![invalid_preference]),
        Err(ChildContextError::InvalidPreference(_))
    ));
    store.close().await;
}

#[test]
fn personal_instruction_body_never_exceeds_aggregate_limit() {
    let project = TempTree::new();
    let source = vec![b'p'; 300];
    project.write("instructions.md", &source);
    for total_max_bytes in [100, 0] {
        let limits = InstructionLimits {
            personal_max_bytes: 300,
            project_max_bytes: 300,
            total_max_bytes,
        };
        let context = OriginalProjectContext::capture_with_policy(
            &project.workspace(),
            Some(project.path()),
            Vec::new(),
            false,
            &limits,
        )
        .unwrap();
        let fact = &context.instruction_sources()[0];
        assert_eq!(
            fact.origin,
            saya_harness::instructions::InstructionOrigin::Personal
        );
        assert_eq!(fact.source_size, source.len() as u64);
        assert_eq!(fact.source_sha256, digest(&source));
        assert!(fact.source_truncated);

        let spec = context
            .project_child(
                authority("personal-aggregate", "src/personal.rs"),
                ChildBrief {
                    task: "use bounded personal context".to_owned(),
                    dependencies: Vec::new(),
                    artifacts: Vec::new(),
                },
                options("model"),
            )
            .unwrap();
        let block = spec
            .request()
            .context_blocks
            .iter()
            .find(|block| block.label == "untrusted-personal-instructions")
            .unwrap();
        let admitted = block
            .body
            .split_once("Untrusted instruction text:\n")
            .unwrap()
            .1;
        assert_eq!(admitted.len(), total_max_bytes);
        assert!(block.truncated);
    }
}

fn digest(bytes: &[u8]) -> Sha256Digest {
    let expected: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Sha256Digest::parse(&expected).unwrap()
}
