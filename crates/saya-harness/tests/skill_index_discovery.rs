#[cfg(unix)]
#[path = "skill_catalog_fixtures/filesystem.rs"]
mod filesystem;
#[cfg(unix)]
#[path = "skill_catalog_fixtures/format.rs"]
mod format;
#[cfg(unix)]
#[path = "skill_catalog_fixtures/limits.rs"]
mod limits;
#[cfg(unix)]
#[path = "skill_catalog_fixtures/precedence.rs"]
mod precedence;
#[path = "skill_catalog_fixtures/support.rs"]
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
