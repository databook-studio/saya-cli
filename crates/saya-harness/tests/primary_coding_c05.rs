#[cfg(unix)]
#[path = "primary_coding_c05/filesystem.rs"]
mod filesystem;
#[cfg(unix)]
#[path = "primary_coding_c05/format.rs"]
mod format;
#[cfg(unix)]
#[path = "primary_coding_c05/limits.rs"]
mod limits;
#[cfg(unix)]
#[path = "primary_coding_c05/precedence.rs"]
mod precedence;
#[path = "primary_coding_c05/support.rs"]
mod support;

#[cfg(not(unix))]
#[test]
fn discovery_fails_closed_off_unix() {
    use saya_harness::coding::skills::{SkillIndexError, SkillOrigin, discover_skills};
    use support::{TempTree, root};

    let tree = TempTree::new();
    assert!(matches!(
        discover_skills(&[root(&tree, SkillOrigin::User, "user")]),
        Err(SkillIndexError::UnsupportedPlatform)
    ));
}
