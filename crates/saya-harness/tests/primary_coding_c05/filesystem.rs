use std::fs;

use saya_harness::coding::skills::{SkillIndexError, SkillOrigin, SkillRoot, discover_skills};

use super::support::{TempTree, root};

#[test]
fn malformed_headers_and_same_origin_duplicates_are_refused() {
    let malformed = TempTree::new();
    let skill_dir = malformed.path().join("bad");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        b"---\nname: bad\nname: duplicate\ndescription: no\n---\n",
    )
    .unwrap();
    assert!(matches!(
        discover_skills(&[root(&malformed, SkillOrigin::User, "user")]),
        Err(SkillIndexError::MalformedFrontmatter)
    ));

    let missing = TempTree::new();
    let skill_dir = missing.path().join("missing");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), b"---\nname: missing\n").unwrap();
    assert!(matches!(
        discover_skills(&[root(&missing, SkillOrigin::User, "user")]),
        Err(SkillIndexError::MalformedFrontmatter)
    ));

    let duplicate_user = TempTree::new();
    let other_user = TempTree::new();
    let duplicate_roots = [
        root(&duplicate_user, SkillOrigin::User, "user one"),
        root(&other_user, SkillOrigin::User, "user two"),
    ];
    assert!(matches!(
        discover_skills(&duplicate_roots),
        Err(SkillIndexError::DuplicateOrigin)
    ));

    let duplicate_skills = TempTree::new();
    duplicate_skills.skill("first", "same", "first", b"");
    duplicate_skills.skill("second", "same", "second", b"");
    assert!(matches!(
        discover_skills(&[root(&duplicate_skills, SkillOrigin::Project, "project")]),
        Err(SkillIndexError::Conflict)
    ));
}

#[cfg(unix)]
#[test]
fn symlink_skill_candidates_are_refused_without_following() {
    use std::os::unix::fs::symlink;

    let outside = TempTree::new();
    outside.skill("real", "real", "outside", b"");
    let root_tree = TempTree::new();
    symlink(
        outside.path().join("real"),
        root_tree.path().join("linked-dir"),
    )
    .unwrap();
    assert!(matches!(
        discover_skills(&[root(&root_tree, SkillOrigin::Project, "project")]),
        Err(SkillIndexError::SymlinkRefused)
    ));

    let root_tree = TempTree::new();
    fs::create_dir(root_tree.path().join("linked-file")).unwrap();
    symlink(
        outside.path().join("real/SKILL.md"),
        root_tree.path().join("linked-file/SKILL.md"),
    )
    .unwrap();
    assert!(matches!(
        discover_skills(&[root(&root_tree, SkillOrigin::Project, "project")]),
        Err(SkillIndexError::SymlinkRefused)
    ));
}

#[test]
fn labels_and_discovered_path_components_are_bounded() {
    let tree = TempTree::new();
    assert!(SkillRoot::new(tree.workspace(), SkillOrigin::User, "a".repeat(128)).is_ok());
    assert!(matches!(
        SkillRoot::new(tree.workspace(), SkillOrigin::User, "a".repeat(129)),
        Err(SkillIndexError::InvalidLabel)
    ));
    assert!(matches!(
        SkillRoot::new(tree.workspace(), SkillOrigin::User, "bad\nlabel"),
        Err(SkillIndexError::InvalidLabel)
    ));

    let path = tree.path().join("bad\nname");
    fs::create_dir(&path).unwrap();
    fs::write(
        path.join("SKILL.md"),
        b"---\nname: invalid-path\ndescription: must fail closed\n---\n",
    )
    .unwrap();
    assert!(matches!(
        discover_skills(&[root(&tree, SkillOrigin::User, "user")]),
        Err(SkillIndexError::InvalidSkillPath)
    ));
}

#[test]
fn ordinary_files_and_directories_without_skill_files_are_skipped() {
    let tree = TempTree::new();
    fs::write(tree.path().join("ordinary.txt"), b"not a skill").unwrap();
    fs::create_dir(tree.path().join("empty-skill-dir")).unwrap();
    let index = discover_skills(&[root(&tree, SkillOrigin::User, "user")]).unwrap();
    assert!(index.skills.is_empty());
}
