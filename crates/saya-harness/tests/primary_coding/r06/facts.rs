use saya_harness::coding::context::{
    ArtifactReference, ChildBrief, ChildContextError, ChildDependency, MAX_CONTEXT_SOURCE_FACTS,
    OriginalProjectContext,
};
use saya_types::{
    PROJECT_PREFERENCE_VERSION, PreferenceId, PreferenceSource, ProjectPreference, Sha256Digest,
    WorktreeId,
};

use super::support::{TempTree, authority, options};

#[test]
fn source_fact_aggregate_accepts_exact_bound_and_refuses_one_more() {
    let project = TempTree::new();
    let workspace = project.workspace();
    let worktree = WorktreeId::from_canonical_path(workspace.root().to_str().unwrap()).unwrap();
    let preferences = (0..64)
        .map(|index| ProjectPreference {
            version: PROJECT_PREFERENCE_VERSION,
            id: PreferenceId::parse(&format!("pp-{index:032x}")).unwrap(),
            worktree: worktree.clone(),
            text: "explicit note".to_owned(),
            source: PreferenceSource::UserExplicit,
            created_unix_ms: index,
            updated_unix_ms: index,
        })
        .collect();
    let context = OriginalProjectContext::capture(&workspace, None, preferences).unwrap();
    let digest = Sha256Digest::parse(&"d".repeat(64)).unwrap();
    let exact_dependencies = (0..8)
        .map(|index| ChildDependency {
            id: format!("dependency-{index}"),
            summary: String::new(),
        })
        .collect();
    let exact_artifacts = (0..8)
        .map(|index| ArtifactReference {
            id: format!("artifact-{index}"),
            sha256: digest.clone(),
        })
        .collect();
    context
        .project_child(
            authority("facts-exact", "src/facts.rs"),
            ChildBrief {
                task: "task".to_owned(),
                dependencies: exact_dependencies,
                artifacts: exact_artifacts,
            },
            options("model"),
        )
        .unwrap();
    assert_eq!(
        context.preference_count()
            + context.instruction_sources().len()
            + context.skipped_sources().len()
            + 8
            + 8,
        MAX_CONTEXT_SOURCE_FACTS
    );

    let too_many_artifacts = (0..9)
        .map(|index| ArtifactReference {
            id: format!("artifact-{index}"),
            sha256: digest.clone(),
        })
        .collect();
    let too_many_dependencies = (0..8)
        .map(|index| ChildDependency {
            id: format!("dependency-{index}"),
            summary: String::new(),
        })
        .collect();
    assert!(matches!(
        context.project_child(
            authority("facts-over", "src/facts.rs"),
            ChildBrief {
                task: "task".to_owned(),
                dependencies: too_many_dependencies,
                artifacts: too_many_artifacts,
            },
            options("model"),
        ),
        Err(ChildContextError::TooManySourceFacts)
    ));
}
