//! Nested discovery: the root file, a `CLAUDE.md` fallback only when
//! `AGENTS.md` is absent, and the `AGENTS.md`/`CLAUDE.md` files in the
//! directories between the workspace root and the current working directory —
//! top-down, bounded, and never through a symlink.

use std::fs;

use super::{CONTENT, LIMITS, Sandbox, load_instructions};
#[cfg(unix)]
use super::{OUTSIDE, no_leak};

use saya_harness::instructions::{
    InstructionLimits, InstructionOrigin, MAX_NESTED_INSTRUCTION_FILES,
};
#[cfg(unix)]
use saya_harness::instructions::{SkipReason, SkippedSource};

fn write(root: &std::path::Path, rel: &str, bytes: &[u8]) {
    let full = root.join(rel);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(full, bytes).unwrap();
}

fn load_at(
    sandbox: &Sandbox,
    working: &std::path::Path,
    limits: &InstructionLimits,
) -> saya_harness::instructions::LoadedInstructions {
    load_instructions(Some(&sandbox.project), Some(working), None, limits)
}

#[test]
fn the_root_falls_back_to_claude_md_only_when_agents_md_is_absent() {
    let sandbox = Sandbox::new("claude-fallback");
    write(&sandbox.project, "CLAUDE.md", CONTENT);

    let loaded = load_at(&sandbox, &sandbox.project, &LIMITS);
    assert!(loaded.skipped.is_empty());
    assert_eq!(loaded.sources.len(), 1);
    assert_eq!(loaded.sources[0].origin, InstructionOrigin::Project);
    assert_eq!(loaded.sources[0].file_name, "CLAUDE.md");
    assert_eq!(loaded.sources[0].path, "CLAUDE.md");
    assert_eq!(loaded.sources[0].body, "inside-sentinel");
}

#[test]
fn agents_md_wins_over_claude_md_in_the_same_directory() {
    let sandbox = Sandbox::new("agents-wins");
    write(&sandbox.project, "AGENTS.md", b"from agents");
    write(&sandbox.project, "CLAUDE.md", b"from claude");

    let loaded = load_at(&sandbox, &sandbox.project, &LIMITS);
    assert!(loaded.skipped.is_empty(), "the loser is ignored silently");
    assert_eq!(loaded.sources.len(), 1);
    assert_eq!(loaded.sources[0].file_name, "AGENTS.md");
    assert_eq!(loaded.sources[0].body, "from agents");
}

#[test]
fn nested_files_load_from_the_root_to_the_working_directory_in_order() {
    let sandbox = Sandbox::new("nested-order");
    write(&sandbox.project, "AGENTS.md", b"root");
    write(&sandbox.project, "sub/AGENTS.md", b"sub");
    write(&sandbox.project, "sub/deep/CLAUDE.md", b"deep");
    write(&sandbox.project, "sub/other/AGENTS.md", b"not an ancestor");

    let loaded = load_at(&sandbox, &sandbox.project.join("sub/deep"), &LIMITS);
    assert!(loaded.skipped.is_empty());
    let paths: Vec<_> = loaded
        .sources
        .iter()
        .map(|source| source.path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["AGENTS.md", "sub/AGENTS.md", "sub/deep/CLAUDE.md"]
    );
    let bodies: Vec<_> = loaded
        .sources
        .iter()
        .map(|source| source.body.as_str())
        .collect();
    assert_eq!(bodies, vec!["root", "sub", "deep"]);
}

#[test]
fn a_working_directory_outside_the_root_loads_only_the_root() {
    let sandbox = Sandbox::new("outside-working");
    write(&sandbox.project, "AGENTS.md", b"root");
    let elsewhere = sandbox.outer.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    write(&elsewhere, "AGENTS.md", b"outside");

    let loaded = load_at(&sandbox, &elsewhere, &LIMITS);
    assert_eq!(loaded.sources.len(), 1);
    assert_eq!(loaded.sources[0].path, "AGENTS.md");
}

#[test]
fn the_nested_file_count_is_bounded() {
    let sandbox = Sandbox::new("nested-bound");
    write(&sandbox.project, "AGENTS.md", b"root");
    let mut working = sandbox.project.clone();
    for index in 0..MAX_NESTED_INSTRUCTION_FILES + 5 {
        working.push(format!("d{index:02}"));
        write(&working, "AGENTS.md", b"nested");
    }

    let loaded = load_at(&sandbox, &working, &LIMITS);
    assert_eq!(
        loaded.sources.len(),
        1 + MAX_NESTED_INSTRUCTION_FILES,
        "the root plus the nested-file bound"
    );
    assert_eq!(loaded.sources[0].path, "AGENTS.md");
    assert!(
        loaded.sources[1].path.ends_with("d00/AGENTS.md"),
        "nested files start at the first subdirectory: {}",
        loaded.sources[1].path
    );
}

#[test]
fn the_total_cap_is_shared_by_root_and_nested_files() {
    let sandbox = Sandbox::new("nested-total-cap");
    let total = 32 * 1024;
    let limits = InstructionLimits {
        personal_max_bytes: 16 * 1024,
        project_max_bytes: total,
        total_max_bytes: total,
    };
    write(&sandbox.project, "AGENTS.md", &vec![b'r'; 20 * 1024]);
    write(&sandbox.project, "sub/AGENTS.md", &vec![b'n'; 20 * 1024]);

    let loaded = load_at(&sandbox, &sandbox.project.join("sub"), &limits);
    assert_eq!(loaded.sources.len(), 2);
    assert_eq!(loaded.sources[0].body.len(), 20 * 1024);
    assert!(!loaded.sources[0].truncated);
    assert_eq!(
        loaded.sources[1].body.len(),
        12 * 1024,
        "the root is admitted first and the nested file takes the remainder"
    );
    assert!(loaded.sources[1].truncated, "the cut is stated");
}

#[test]
fn nested_files_share_the_project_allowance_even_when_the_total_is_larger() {
    let sandbox = Sandbox::new("nested-project-cap");
    let project = 32 * 1024;
    let limits = InstructionLimits {
        personal_max_bytes: 16 * 1024,
        project_max_bytes: project,
        total_max_bytes: 48 * 1024,
    };
    write(&sandbox.project, "AGENTS.md", &vec![b'r'; 20 * 1024]);
    write(&sandbox.project, "sub/AGENTS.md", &vec![b'n'; 20 * 1024]);

    let loaded = load_at(&sandbox, &sandbox.project.join("sub"), &limits);
    assert_eq!(loaded.sources.len(), 2);
    assert_eq!(loaded.sources[0].body.len(), 20 * 1024);
    assert_eq!(
        loaded.sources[1].body.len(),
        12 * 1024,
        "project files together stay inside the 32 KiB project allowance"
    );
    assert!(loaded.sources[1].truncated, "the cut is stated");
}

#[test]
#[cfg(unix)]
fn a_symlinked_nested_file_is_refused_and_outside_never_leaks() {
    use std::os::unix::fs::symlink;

    let sandbox = Sandbox::new("nested-symlink");
    write(&sandbox.project, "AGENTS.md", CONTENT);
    fs::create_dir_all(sandbox.project.join("sub")).unwrap();
    let outside = sandbox.outer.join("outside.txt");
    fs::write(&outside, OUTSIDE).unwrap();
    symlink(&outside, sandbox.project.join("sub/AGENTS.md")).unwrap();

    let loaded = load_at(&sandbox, &sandbox.project.join("sub"), &LIMITS);
    assert_eq!(loaded.sources.len(), 1, "only the root file loads");
    assert_eq!(
        loaded.skipped,
        vec![SkippedSource {
            origin: InstructionOrigin::Project,
            file_name: "AGENTS.md",
            reason: SkipReason::SymlinkRefused,
        }]
    );
    no_leak(&loaded);
}
