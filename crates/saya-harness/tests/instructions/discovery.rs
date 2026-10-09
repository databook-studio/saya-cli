//! Which named sources load, in what order, and what the caller passes:
//! the two-root selection, the `None` disables, determinism across calls,
//! the typed path arguments, and the directory-shape errors at the root.

use std::{fs, path::Path};

use super::{CONTENT, KNOWN_DIGEST, LIMITS, Sandbox, load, write_personal, write_project};

use saya_harness::instructions::{InstructionOrigin, SkipReason, SkippedSource, load_instructions};

#[test]
fn both_missing_yields_nothing() {
    let sandbox = Sandbox::new("empty");
    let loaded = load(&sandbox, &LIMITS);
    assert!(loaded.sources.is_empty());
    assert!(loaded.skipped.is_empty(), "a missing file is silent");
}

#[test]
fn project_only_loads_the_root_file() {
    let sandbox = Sandbox::new("project-only");
    write_project(&sandbox, CONTENT);

    let loaded = load_instructions(Some(&sandbox.project), None, &LIMITS);
    assert!(loaded.skipped.is_empty());
    assert_eq!(loaded.sources.len(), 1);
    let source = &loaded.sources[0];
    assert_eq!(source.origin, InstructionOrigin::Project);
    assert_eq!(source.file_name, "AGENTS.md");
    assert_eq!(source.body, "inside-sentinel");
    assert_eq!(source.sha256, KNOWN_DIGEST);
    assert_eq!(source.size, CONTENT.len() as u64);
    assert!(!source.truncated);
}

#[test]
fn both_present_orders_personal_then_project() {
    let sandbox = Sandbox::new("both");
    write_personal(&sandbox, CONTENT);
    write_project(&sandbox, CONTENT);

    let loaded = load(&sandbox, &LIMITS);
    assert!(loaded.skipped.is_empty());
    assert_eq!(loaded.sources.len(), 2);
    assert_eq!(loaded.sources[0].origin, InstructionOrigin::Personal);
    assert_eq!(loaded.sources[0].file_name, "instructions.md");
    assert_eq!(loaded.sources[0].body, "inside-sentinel");
    assert_eq!(loaded.sources[0].sha256, KNOWN_DIGEST);
    assert_eq!(loaded.sources[1].origin, InstructionOrigin::Project);
    assert_eq!(loaded.sources[1].file_name, "AGENTS.md");
    assert_eq!(loaded.sources[1].sha256, KNOWN_DIGEST);
}

#[test]
fn none_disables_a_source_entirely() {
    let sandbox = Sandbox::new("disabled");
    write_personal(&sandbox, CONTENT);
    write_project(&sandbox, CONTENT);

    let personal_only = load_instructions(None, Some(&sandbox.personal), &LIMITS);
    assert!(personal_only.skipped.is_empty());
    assert_eq!(personal_only.sources.len(), 1);
    assert_eq!(personal_only.sources[0].origin, InstructionOrigin::Personal);
    let project_only = load_instructions(Some(&sandbox.project), None, &LIMITS);
    assert!(project_only.skipped.is_empty());
    assert_eq!(project_only.sources.len(), 1);
    assert_eq!(project_only.sources[0].origin, InstructionOrigin::Project);
}

#[test]
fn loading_is_deterministic() {
    let sandbox = Sandbox::new("deterministic");
    write_personal(&sandbox, CONTENT);
    write_project(&sandbox, CONTENT);

    let first = load(&sandbox, &LIMITS);
    let second = load(&sandbox, &LIMITS);
    assert_eq!(first, second);
}

#[test]
fn path_typed_args_flow_through() {
    let sandbox = Sandbox::new("types");
    write_project(&sandbox, CONTENT);
    let project: &Path = &sandbox.project;
    let loaded = load_instructions(Some(project), None, &LIMITS);
    assert_eq!(loaded.sources.len(), 1);
}

#[test]
fn a_missing_personal_directory_is_silent() {
    let sandbox = Sandbox::new("missing-personal-dir");
    let absent = sandbox.outer.join("does-not-exist");
    let loaded = load_instructions(None, Some(&absent), &LIMITS);
    assert!(loaded.sources.is_empty());
    assert!(
        loaded.skipped.is_empty(),
        "a missing directory is absent, not unreadable: {loaded:?}"
    );
}

#[test]
fn a_personal_dir_that_is_a_file_is_unreadable() {
    let sandbox = Sandbox::new("personal-dir-file");
    let file = sandbox.outer.join("not-a-dir");
    fs::write(&file, CONTENT).unwrap();
    let loaded = load_instructions(None, Some(&file), &LIMITS);
    assert!(loaded.sources.is_empty());
    assert_eq!(
        loaded.skipped,
        vec![SkippedSource {
            origin: InstructionOrigin::Personal,
            reason: SkipReason::Unreadable,
        }]
    );
}
