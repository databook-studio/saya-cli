use saya_harness::coding::context::{
    ArtifactReference, ChildBrief, ChildContextError, ChildDependency,
    MAX_CHILD_ARTIFACT_REFERENCES, MAX_CHILD_DEPENDENCIES, MAX_CHILD_DEPENDENCY_SUMMARY_BYTES,
    MAX_CHILD_TASK_BYTES, OriginalProjectContext,
};
use saya_types::Sha256Digest;

use super::support::{TempTree, authority, options};

fn context() -> OriginalProjectContext {
    let project = TempTree::new();
    OriginalProjectContext::capture(&project.workspace(), None, Vec::new()).unwrap()
}

fn brief(task: String) -> ChildBrief {
    ChildBrief {
        task,
        dependencies: Vec::new(),
        artifacts: Vec::new(),
    }
}

#[test]
fn task_and_dependency_bounds_accept_exact_and_refuse_one_over() {
    let context = context();
    let exact_task = brief("t".repeat(MAX_CHILD_TASK_BYTES));
    assert!(
        context
            .project_child(
                authority("exact-task", "src/exact.rs"),
                exact_task,
                options("model"),
            )
            .is_ok()
    );
    assert!(matches!(
        context.project_child(
            authority("long-task", "src/long.rs"),
            brief("t".repeat(MAX_CHILD_TASK_BYTES + 1)),
            options("model"),
        ),
        Err(ChildContextError::TaskTooLarge)
    ));

    let exact_dependencies = (0..MAX_CHILD_DEPENDENCIES)
        .map(|index| ChildDependency {
            id: format!("dependency-{index}"),
            summary: "s".repeat(MAX_CHILD_DEPENDENCY_SUMMARY_BYTES),
        })
        .collect();
    let dependencies = context
        .project_child(
            authority("exact-dependencies", "src/dependencies.rs"),
            ChildBrief {
                task: "task".to_owned(),
                dependencies: exact_dependencies,
                artifacts: Vec::new(),
            },
            options("model"),
        )
        .unwrap();
    assert!(dependencies.request().context_blocks.len() <= 32);

    let too_many_dependencies = (0..MAX_CHILD_DEPENDENCIES + 1)
        .map(|index| ChildDependency {
            id: format!("dependency-{index}"),
            summary: String::new(),
        })
        .collect();
    assert!(matches!(
        context.project_child(
            authority("too-many-dependencies", "src/dependencies.rs"),
            ChildBrief {
                task: "task".to_owned(),
                dependencies: too_many_dependencies,
                artifacts: Vec::new(),
            },
            options("model"),
        ),
        Err(ChildContextError::TooManyDependencies)
    ));

    let long_summary = ChildBrief {
        task: "task".to_owned(),
        dependencies: vec![ChildDependency {
            id: "dependency".to_owned(),
            summary: "s".repeat(MAX_CHILD_DEPENDENCY_SUMMARY_BYTES + 1),
        }],
        artifacts: Vec::new(),
    };
    assert!(matches!(
        context.project_child(
            authority("long-summary", "src/dependency.rs"),
            long_summary,
            options("model"),
        ),
        Err(ChildContextError::DependencySummaryTooLarge)
    ));
}

#[test]
fn artifact_reference_count_and_opaque_id_bounds_are_enforced() {
    let context = context();
    let digest = Sha256Digest::parse(&"b".repeat(64)).unwrap();
    let artifacts = (0..MAX_CHILD_ARTIFACT_REFERENCES)
        .map(|index| ArtifactReference {
            id: format!("artifact-{index}"),
            sha256: digest.clone(),
        })
        .collect();
    assert!(
        context
            .project_child(
                authority("exact-artifacts", "src/artifacts.rs"),
                ChildBrief {
                    task: "task".to_owned(),
                    dependencies: Vec::new(),
                    artifacts,
                },
                options("model"),
            )
            .is_ok()
    );

    let too_many = (0..MAX_CHILD_ARTIFACT_REFERENCES + 1)
        .map(|index| ArtifactReference {
            id: format!("artifact-{index}"),
            sha256: digest.clone(),
        })
        .collect();
    assert!(matches!(
        context.project_child(
            authority("too-many-artifacts", "src/artifacts.rs"),
            ChildBrief {
                task: "task".to_owned(),
                dependencies: Vec::new(),
                artifacts: too_many,
            },
            options("model"),
        ),
        Err(ChildContextError::TooManyArtifacts)
    ));

    let max_id = ChildBrief {
        task: "task".to_owned(),
        dependencies: Vec::new(),
        artifacts: vec![ArtifactReference {
            id: "a".repeat(128),
            sha256: digest.clone(),
        }],
    };
    assert!(
        context
            .project_child(authority("max-ref", "src/max.rs"), max_id, options("model"))
            .is_ok()
    );
    let too_long_id = ChildBrief {
        task: "task".to_owned(),
        dependencies: Vec::new(),
        artifacts: vec![ArtifactReference {
            id: "a".repeat(129),
            sha256: digest,
        }],
    };
    assert!(matches!(
        context.project_child(
            authority("long-ref", "src/long.rs"),
            too_long_id,
            options("model"),
        ),
        Err(ChildContextError::InvalidArtifactReference)
    ));
    let path_like_id = ChildBrief {
        task: "task".to_owned(),
        dependencies: Vec::new(),
        artifacts: vec![ArtifactReference {
            id: "../artifact.bin".to_owned(),
            sha256: Sha256Digest::parse(&"c".repeat(64)).unwrap(),
        }],
    };
    assert!(matches!(
        context.project_child(
            authority("path-ref", "src/path.rs"),
            path_like_id,
            options("model"),
        ),
        Err(ChildContextError::InvalidArtifactReference)
    ));
}
