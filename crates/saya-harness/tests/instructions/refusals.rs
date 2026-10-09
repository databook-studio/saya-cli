//! The typed refusals, recorded as skips: a symlinked root refused without
//! being read, a directory that is not a regular file, invalid UTF-8
//! refused whole, and the same mapping by origin for the personal root.

use std::fs;

use super::{CONTENT, LIMITS, OUTSIDE, Sandbox, load, no_leak, write_project};

use saya_harness::instructions::{InstructionOrigin, SkipReason, SkippedSource, load_instructions};

#[test]
#[cfg(unix)]
fn symlinked_agents_is_refused_and_outside_never_leaks() {
    use std::os::unix::fs::symlink;

    let sandbox = Sandbox::new("symlink");
    let outside = sandbox.outer.join("outside.txt");
    fs::write(&outside, OUTSIDE).unwrap();
    symlink(&outside, sandbox.project.join("AGENTS.md")).unwrap();

    let loaded = load_instructions(Some(&sandbox.project), None, &LIMITS);
    assert!(loaded.sources.is_empty());
    assert_eq!(
        loaded.skipped,
        vec![SkippedSource {
            origin: InstructionOrigin::Project,
            reason: SkipReason::SymlinkRefused,
        }]
    );
    no_leak(&loaded);
    assert_eq!(fs::read(&outside).unwrap(), OUTSIDE, "outside untouched");
}

#[test]
fn directory_named_agents_is_not_a_regular_file() {
    let sandbox = Sandbox::new("directory");
    fs::create_dir_all(sandbox.project.join("AGENTS.md")).unwrap();

    let loaded = load_instructions(Some(&sandbox.project), None, &LIMITS);
    assert!(loaded.sources.is_empty());
    assert_eq!(
        loaded.skipped,
        vec![SkippedSource {
            origin: InstructionOrigin::Project,
            reason: SkipReason::NotRegularFile,
        }]
    );
}

#[test]
fn invalid_utf8_is_skipped() {
    let sandbox = Sandbox::new("utf8");
    write_project(&sandbox, b"ok\xff\xffbad");
    let loaded = load_instructions(Some(&sandbox.project), None, &LIMITS);
    assert!(loaded.sources.is_empty());
    assert_eq!(
        loaded.skipped,
        vec![SkippedSource {
            origin: InstructionOrigin::Project,
            reason: SkipReason::NotUtf8,
        }]
    );

    // An incomplete trailing sequence the cap did NOT cut is the file's own
    // defect: the whole source is refused, not repaired.
    write_project(&sandbox, b"ok\xc3");
    let loaded = load_instructions(Some(&sandbox.project), None, &LIMITS);
    assert!(loaded.sources.is_empty());
    assert_eq!(
        loaded.skipped,
        vec![SkippedSource {
            origin: InstructionOrigin::Project,
            reason: SkipReason::NotUtf8,
        }]
    );
}

#[test]
#[cfg(unix)]
fn personal_symlink_maps_by_origin_too() {
    use std::os::unix::fs::symlink;

    let sandbox = Sandbox::new("personal-link");
    let outside = sandbox.outer.join("outside.txt");
    fs::write(&outside, OUTSIDE).unwrap();
    symlink(&outside, sandbox.personal.join("instructions.md")).unwrap();
    write_project(&sandbox, CONTENT);

    let loaded = load(&sandbox, &LIMITS);
    assert_eq!(
        loaded.skipped,
        vec![SkippedSource {
            origin: InstructionOrigin::Personal,
            reason: SkipReason::SymlinkRefused,
        }]
    );
    assert_eq!(loaded.sources.len(), 1, "project still loads");
    no_leak(&loaded);
}
