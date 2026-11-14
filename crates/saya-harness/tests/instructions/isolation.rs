//! The loader's bounded surface: a sibling directory outside the working
//! path is never discovered, and an include directive is literal text whose
//! target is never read — the outside sentinel never leaks into a body.

use std::fs;

use super::{CONTENT, LIMITS, OUTSIDE, Sandbox, no_leak, write_project};

use saya_harness::instructions::load_instructions;

#[test]
fn a_sibling_directory_is_never_discovered() {
    let sandbox = Sandbox::new("no-walk");
    fs::create_dir_all(sandbox.project.join("sub")).unwrap();
    fs::write(sandbox.project.join("sub/AGENTS.md"), CONTENT).unwrap();

    let loaded = load_instructions(
        Some(&sandbox.project),
        Some(&sandbox.project),
        None,
        &LIMITS,
    );
    assert!(
        loaded.sources.is_empty(),
        "the walk stops at the working directory, never into a sibling"
    );
    assert!(
        loaded.skipped.is_empty(),
        "a missing root file is silent — the sibling one is never attempted"
    );
}

#[test]
fn include_directive_is_literal_and_other_files_are_not_read() {
    let sandbox = Sandbox::new("literal");
    write_project(&sandbox, b"see @other.md for details\n");
    fs::write(sandbox.project.join("other.md"), OUTSIDE).unwrap();

    let loaded = load_instructions(
        Some(&sandbox.project),
        Some(&sandbox.project),
        None,
        &LIMITS,
    );
    assert!(loaded.skipped.is_empty());
    assert_eq!(loaded.sources.len(), 1);
    assert_eq!(loaded.sources[0].body, "see @other.md for details\n");
    no_leak(&loaded);
}
