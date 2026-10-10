use std::fs;

use saya_harness::coding::skills::{SkillIndexError, SkillOrigin, discover_skills};

use super::support::{TempTree, root};

#[test]
fn required_metadata_supports_simple_yaml_description_blocks() {
    let tree = TempTree::new();
    let directory = tree.path().join("blocks");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("SKILL.md"),
        b"---\nname: \"blocks\"\ndescription: >\n  first line\n  second line\nargument-hint: mode-lite\nlicense: MIT\n---\nbody\n",
    )
    .unwrap();

    let index = discover_skills(&[root(&tree, SkillOrigin::Project, "project")]).unwrap();
    assert_eq!(index.skills[0].name, "blocks");
    assert_eq!(index.skills[0].description, "first line second line\n");

    let literal = tree.path().join("literal");
    fs::create_dir_all(&literal).unwrap();
    fs::write(
        literal.join("SKILL.md"),
        b"---\nname: literal\ndescription: |\n  first line\n  second line\n---\nbody\n",
    )
    .unwrap();
    let index = discover_skills(&[root(&tree, SkillOrigin::Project, "project")]).unwrap();
    assert_eq!(index.skills[1].name, "literal");
    assert_eq!(index.skills[1].description, "first line\nsecond line\n");
}

#[test]
fn optional_scalar_metadata_is_ignored_but_bounded() {
    let exact = TempTree::new();
    let directory = exact.path().join("optional");
    fs::create_dir(&directory).unwrap();
    let mut source = b"---\nname: optional\ndescription: valid\nlicense: ".to_vec();
    source.extend(std::iter::repeat_n(b'a', 8 * 1024));
    source.extend_from_slice(b"\n---\n");
    fs::write(directory.join("SKILL.md"), source).unwrap();
    let index = discover_skills(&[root(&exact, SkillOrigin::User, "user")]).unwrap();
    assert_eq!(index.skills[0].description, "valid");

    let too_large = TempTree::new();
    let directory = too_large.path().join("optional");
    fs::create_dir(&directory).unwrap();
    let mut source = b"---\nname: optional\ndescription: valid\nlicense: ".to_vec();
    source.extend(std::iter::repeat_n(b'a', 8 * 1024 + 1));
    source.extend_from_slice(b"\n---\n");
    fs::write(directory.join("SKILL.md"), source).unwrap();
    assert!(matches!(
        discover_skills(&[root(&too_large, SkillOrigin::User, "user")]),
        Err(SkillIndexError::MalformedFrontmatter)
    ));
}

#[test]
fn unsupported_yaml_constructs_are_refused() {
    for (directory_name, description) in [
        ("alias", "*untrusted"),
        ("flow", "[untrusted, value]"),
        ("tag", "!str untrusted"),
        ("modifier", ">-"),
    ] {
        let tree = TempTree::new();
        let directory = tree.path().join(directory_name);
        fs::create_dir(&directory).unwrap();
        let source = format!("---\nname: {directory_name}\ndescription: {description}\n---\n");
        fs::write(directory.join("SKILL.md"), source.as_bytes()).unwrap();
        assert!(matches!(
            discover_skills(&[root(&tree, SkillOrigin::User, "user")]),
            Err(SkillIndexError::MalformedFrontmatter)
        ));
    }
}
