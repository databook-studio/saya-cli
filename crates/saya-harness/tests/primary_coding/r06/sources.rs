use std::fs;

use saya_harness::coding::context::{
    ChildBrief, MAX_INSTRUCTION_BYTES, MAX_PERSONAL_INSTRUCTION_BYTES,
    MAX_PROJECT_INSTRUCTION_BYTES, OriginalProjectContext,
};
use saya_harness::instructions::InstructionLimits;
use saya_harness::instructions::{InstructionOrigin, SkipReason};
use saya_types::Sha256Digest;
use sha2::{Digest, Sha256};

use super::support::{TempTree, authority, options};

#[cfg(unix)]
#[test]
fn disabled_project_instruction_source_is_not_loaded() {
    use std::os::unix::fs::symlink;

    let outside = TempTree::new();
    outside.write(
        "AGENTS.md",
        b"must not be read while project instructions are disabled\n",
    );
    let project = TempTree::new();
    symlink(
        outside.path().join("AGENTS.md"),
        project.path().join("AGENTS.md"),
    )
    .unwrap();
    let limits = InstructionLimits {
        personal_max_bytes: MAX_PERSONAL_INSTRUCTION_BYTES,
        project_max_bytes: MAX_PROJECT_INSTRUCTION_BYTES,
        total_max_bytes: MAX_INSTRUCTION_BYTES,
    };

    let context = OriginalProjectContext::capture_with_policy(
        &project.workspace(),
        None,
        Vec::new(),
        false,
        &limits,
    )
    .unwrap();
    assert!(context.instruction_sources().is_empty());
    assert!(context.skipped_sources().is_empty());
}

#[cfg(unix)]
#[test]
fn skipped_original_source_facts_are_typed_without_paths_or_bodies() {
    use std::os::unix::fs::symlink;

    let outside = TempTree::new();
    outside.write("AGENTS.md", b"outside instruction body marker\n");
    let project = TempTree::new();
    symlink(
        outside.path().join("AGENTS.md"),
        project.path().join("AGENTS.md"),
    )
    .unwrap();

    let context = OriginalProjectContext::capture(&project.workspace(), None, Vec::new()).unwrap();
    assert!(context.instruction_sources().is_empty());
    assert_eq!(context.skipped_sources().len(), 1);
    assert_eq!(
        context.skipped_sources()[0].origin,
        InstructionOrigin::Project
    );
    assert_eq!(
        context.skipped_sources()[0].reason,
        SkipReason::SymlinkRefused
    );
    let debug = format!("{context:?}");
    assert!(!debug.contains("outside instruction body marker"));
    assert!(!debug.contains(project.path().to_str().unwrap()));
}

#[test]
fn invalid_utf8_original_instruction_is_a_typed_skip() {
    let project = TempTree::new();
    fs::write(project.path().join("AGENTS.md"), [0xff, 0xfe]).unwrap();

    let context = OriginalProjectContext::capture(&project.workspace(), None, Vec::new()).unwrap();
    assert!(context.instruction_sources().is_empty());
    assert_eq!(context.skipped_sources().len(), 1);
    assert_eq!(
        context.skipped_sources()[0].origin,
        InstructionOrigin::Project
    );
    assert_eq!(context.skipped_sources()[0].reason, SkipReason::NotUtf8);
}

#[test]
fn truncated_instruction_fact_keeps_full_source_digest_and_is_labelled() {
    let project = TempTree::new();
    let mut source = vec![b'a'; 8 * 1024];
    source.extend_from_slice(b"tail-marker");
    project.write("AGENTS.md", &source);
    let expected = digest(&source);

    let context = OriginalProjectContext::capture(&project.workspace(), None, Vec::new()).unwrap();
    let fact = &context.instruction_sources()[0];
    assert!(fact.source_truncated);
    assert_eq!(fact.source_size, source.len() as u64);
    assert_eq!(fact.source_sha256, expected);
    assert!(!format!("{fact:?}").contains("tail-marker"));

    let spec = context
        .project_child(
            authority("truncated-source", "src/truncated.rs"),
            ChildBrief {
                task: "summarize instruction source".to_owned(),
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
        .find(|block| block.label == "untrusted-project-instructions")
        .unwrap();
    assert!(block.truncated);
    assert!(block.body.contains("source_truncated=true"));
    assert!(!block.body.contains("tail-marker"));
}

#[test]
fn lowered_instruction_limits_keep_full_source_provenance_and_projection_truncation() {
    let project = TempTree::new();
    let personal = TempTree::new();
    let personal_source = vec![b'p'; 350];
    let project_source = vec![b'g'; 500];
    personal.write("instructions.md", &personal_source);
    project.write("AGENTS.md", &project_source);
    let limits = InstructionLimits {
        personal_max_bytes: 300,
        project_max_bytes: 350,
        total_max_bytes: 400,
    };

    let context = OriginalProjectContext::capture_with_policy(
        &project.workspace(),
        Some(personal.path()),
        Vec::new(),
        true,
        &limits,
    )
    .unwrap();
    let facts = context.instruction_sources();
    assert_eq!(facts.len(), 2);
    assert_eq!(facts[0].origin, InstructionOrigin::Personal);
    assert_eq!(facts[0].source_size, personal_source.len() as u64);
    assert!(facts[0].source_truncated);
    assert_eq!(facts[0].source_sha256, digest(&personal_source));
    assert_eq!(facts[1].origin, InstructionOrigin::Project);
    assert_eq!(facts[1].source_size, project_source.len() as u64);
    assert!(facts[1].source_truncated);
    assert_eq!(facts[1].source_sha256, digest(&project_source));

    let spec = context
        .project_child(
            authority("lowered-instructions", "src/lowered.rs"),
            ChildBrief {
                task: "inspect bounded instruction context".to_owned(),
                dependencies: Vec::new(),
                artifacts: Vec::new(),
            },
            options("model"),
        )
        .unwrap();
    let blocks = &spec.request().context_blocks;
    let personal_block = blocks
        .iter()
        .find(|block| block.label == "untrusted-personal-instructions")
        .unwrap();
    let project_block = blocks
        .iter()
        .find(|block| block.label == "untrusted-project-instructions")
        .unwrap();
    assert_eq!(
        personal_block
            .body
            .split_once("Untrusted instruction text:\n")
            .unwrap()
            .1
            .len(),
        300
    );
    assert_eq!(
        project_block
            .body
            .split_once("Untrusted instruction text:\n")
            .unwrap()
            .1
            .len(),
        100
    );
    assert!(personal_block.truncated);
    assert!(project_block.truncated);

    let project_only = OriginalProjectContext::capture_with_policy(
        &project.workspace(),
        None,
        Vec::new(),
        true,
        &limits,
    )
    .unwrap();
    assert_eq!(
        project_only.instruction_sources()[0].origin,
        InstructionOrigin::Project
    );
    let spec = project_only
        .project_child(
            authority("project-instruction-cap", "src/project-cap.rs"),
            ChildBrief {
                task: "inspect per-file instruction cap".to_owned(),
                dependencies: Vec::new(),
                artifacts: Vec::new(),
            },
            options("model"),
        )
        .unwrap();
    let project_block = spec
        .request()
        .context_blocks
        .iter()
        .find(|block| block.label == "untrusted-project-instructions")
        .unwrap();
    assert_eq!(
        project_block
            .body
            .split_once("Untrusted instruction text:\n")
            .unwrap()
            .1
            .len(),
        350
    );
    assert!(project_block.truncated);
}

fn digest(bytes: &[u8]) -> Sha256Digest {
    let expected: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Sha256Digest::parse(&expected).unwrap()
}
