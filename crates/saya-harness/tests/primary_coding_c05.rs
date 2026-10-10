#[cfg(unix)]
mod filesystem;
#[cfg(unix)]
mod format;
#[cfg(unix)]
mod limits;
#[cfg(unix)]
mod precedence;
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
