//! The bounded instruction loader battery: at most two named files are ever
//! read — never through a symlink, never a discovery walk, never an import —
//! with caps, truncation, and whole-file identity honoured as data.

use std::{
    fs,
    path::{Path, PathBuf},
};

use saya_harness::instructions::{
    InstructionLimits, InstructionOrigin, InstructionSource, LoadedInstructions, SkipReason,
    SkippedSource, load_instructions,
};
use sha2::{Digest, Sha256};

const CONTENT: &[u8] = b"inside-sentinel";
const KNOWN_DIGEST: &str = "b568f317d963edff551663a3b460d49899fa97a2d313bb50078916da9e74d15f";
const OUTSIDE: &[u8] = b"outside-sentinel-MUST-NOT-LEAK";

const LIMITS: InstructionLimits = InstructionLimits {
    project_max_bytes: 32 * 1024,
    personal_max_bytes: 16 * 1024,
    total_max_bytes: 48 * 1024,
};

struct Sandbox {
    outer: PathBuf,
    project: PathBuf,
    personal: PathBuf,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-instructions-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        let project = outer.join("project");
        let personal = outer.join("personal");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&personal).unwrap();
        Self {
            outer,
            project,
            personal,
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

fn write_project(sandbox: &Sandbox, bytes: &[u8]) {
    fs::write(sandbox.project.join("AGENTS.md"), bytes).unwrap();
}

fn write_personal(sandbox: &Sandbox, bytes: &[u8]) {
    fs::write(sandbox.personal.join("instructions.md"), bytes).unwrap();
}

fn load(sandbox: &Sandbox, limits: &InstructionLimits) -> LoadedInstructions {
    load_instructions(Some(&sandbox.project), Some(&sandbox.personal), limits)
}

fn source_of(loaded: &LoadedInstructions, origin: InstructionOrigin) -> &InstructionSource {
    loaded
        .sources
        .iter()
        .find(|source| source.origin == origin)
        .expect("source must be present")
}

fn no_leak(loaded: &LoadedInstructions) {
    let sentinel = std::str::from_utf8(OUTSIDE).unwrap();
    for source in &loaded.sources {
        assert!(
            !source.body.contains(sentinel),
            "outside sentinel must never appear in a body"
        );
    }
}

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
fn oversize_multi_byte_cut_stays_valid_utf8() {
    let sandbox = Sandbox::new("straddle");
    // 16 KiB cap; the file puts the two bytes of 'é' across the cap so the
    // cut lands mid-character.
    let cap = 16 * 1024;
    let mut bytes = vec![b'a'; cap - 1];
    bytes.extend_from_slice("é".as_bytes());
    write_project(&sandbox, &bytes);

    let limits = InstructionLimits {
        project_max_bytes: cap,
        ..LIMITS
    };
    let loaded = load_instructions(Some(&sandbox.project), None, &limits);
    assert!(loaded.skipped.is_empty());
    let source = source_of(&loaded, InstructionOrigin::Project);
    assert!(source.truncated);
    assert_eq!(source.size, bytes.len() as u64, "size is the whole file");
    assert_eq!(
        source.body.len(),
        cap - 1,
        "the mid-character cut is dropped"
    );
    assert!(source.body.bytes().all(|byte| byte == b'a'));
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    assert_eq!(source.sha256, format!("{:x}", hasher.finalize()));
}

#[test]
fn aggregate_cap_trims_project_below_its_own_cap() {
    let sandbox = Sandbox::new("aggregate");
    write_personal(&sandbox, &vec![b'p'; 10 * 1024]);
    write_project(&sandbox, &vec![b'q'; 40 * 1024]);

    let loaded = load(&sandbox, &LIMITS);
    let personal = source_of(&loaded, InstructionOrigin::Personal);
    let project = source_of(&loaded, InstructionOrigin::Project);
    assert_eq!(personal.body.len(), 10 * 1024);
    assert_eq!(project.body.len(), 32 * 1024, "min(32 KiB, 48-10 KiB)");
    assert!(project.body.len() <= 38 * 1024);
    assert!(personal.body.len() + project.body.len() <= 48 * 1024);
    assert!(project.truncated);

    // With the per-project cap raised, the aggregate is what binds.
    let wide = InstructionLimits {
        project_max_bytes: 64 * 1024,
        ..LIMITS
    };
    let loaded = load(&sandbox, &wide);
    let project = source_of(&loaded, InstructionOrigin::Project);
    assert_eq!(project.body.len(), 38 * 1024, "48 KiB total minus 10 KiB");
    assert!(project.truncated);
}

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
fn nested_sub_agents_is_never_discovered() {
    let sandbox = Sandbox::new("no-walk");
    fs::create_dir_all(sandbox.project.join("sub")).unwrap();
    fs::write(sandbox.project.join("sub/AGENTS.md"), CONTENT).unwrap();

    let loaded = load_instructions(Some(&sandbox.project), None, &LIMITS);
    assert!(
        loaded.sources.is_empty(),
        "no walk: only the root name is read"
    );
    assert!(
        loaded.skipped.is_empty(),
        "a missing root file is silent — the nested one is never attempted"
    );
}

#[test]
fn include_directive_is_literal_and_other_files_are_not_read() {
    let sandbox = Sandbox::new("literal");
    write_project(&sandbox, b"see @other.md for details\n");
    fs::write(sandbox.project.join("other.md"), OUTSIDE).unwrap();

    let loaded = load_instructions(Some(&sandbox.project), None, &LIMITS);
    assert!(loaded.skipped.is_empty());
    assert_eq!(loaded.sources.len(), 1);
    assert_eq!(loaded.sources[0].body, "see @other.md for details\n");
    no_leak(&loaded);
}

#[test]
fn file_over_the_io_backstop_is_too_large() {
    let sandbox = Sandbox::new("backstop");
    write_project(&sandbox, &vec![b'x'; 8 * 1024 * 1024 + 1]);

    let loaded = load_instructions(Some(&sandbox.project), None, &LIMITS);
    assert!(loaded.sources.is_empty());
    assert_eq!(
        loaded.skipped,
        vec![SkippedSource {
            origin: InstructionOrigin::Project,
            reason: SkipReason::TooLarge,
        }]
    );
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
