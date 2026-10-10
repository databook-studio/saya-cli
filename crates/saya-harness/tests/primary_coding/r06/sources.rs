use std::fs;

use saya_harness::coding::context::{ChildBrief, OriginalProjectContext};
use saya_harness::instructions::{InstructionOrigin, SkipReason};
use saya_types::Sha256Digest;
use sha2::{Digest, Sha256};

use super::support::{TempTree, authority, options};

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
    let expected: String = Sha256::digest(&source)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let expected = Sha256Digest::parse(&expected).unwrap();

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
