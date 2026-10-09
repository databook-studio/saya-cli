//! The caps and their truncation: a per-file cut that stays valid UTF-8,
//! the aggregate total trimming a source below its own cap, and the whole
//! I/O backstop refusing an oversize file.

use sha2::{Digest, Sha256};

use super::{LIMITS, Sandbox, load, source_of, write_personal, write_project};

use saya_harness::instructions::{
    InstructionLimits, InstructionOrigin, SkipReason, SkippedSource, load_instructions,
};

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
