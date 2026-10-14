use std::fs;

use saya_harness::coding::skills::{SkillIndexError, SkillOrigin, discover_skills};

use super::support::{TempTree, root};

#[test]
fn metadata_name_and_description_limits_are_enforced() {
    let accepted = TempTree::new();
    accepted.skill(&"a".repeat(64), &"a".repeat(64), "valid", b"");
    assert_eq!(
        discover_skills(&[root(&accepted, SkillOrigin::User, "user")])
            .unwrap()
            .skills[0]
            .name
            .len(),
        64
    );

    let long_name = TempTree::new();
    long_name.skill(&"a".repeat(65), &"a".repeat(65), "invalid", b"");
    assert!(matches!(
        discover_skills(&[root(&long_name, SkillOrigin::User, "user")]),
        Err(SkillIndexError::MalformedFrontmatter)
    ));

    let uppercase_name = TempTree::new();
    uppercase_name.skill("upper", "Uppercase", "invalid", b"");
    assert!(matches!(
        discover_skills(&[root(&uppercase_name, SkillOrigin::User, "user")]),
        Err(SkillIndexError::MalformedFrontmatter)
    ));

    let long_description = TempTree::new();
    long_description.skill("long", "long", &"a".repeat(8 * 1024 + 1), b"");
    assert!(matches!(
        discover_skills(&[root(&long_description, SkillOrigin::User, "user")]),
        Err(SkillIndexError::MalformedFrontmatter)
    ));
}

#[test]
fn exact_header_limit_is_accepted_and_one_byte_over_is_refused() {
    let exact = TempTree::new();
    let skill_dir = exact.path().join("exact");
    fs::create_dir_all(&skill_dir).unwrap();
    let prefix = b"---\nname: exact\ndescription: ";
    let optional = b"\nargument-hint: ";
    let suffix = b"\n---\n";
    let description_len = 8 * 1024;
    let optional_len = 16 * 1024 - prefix.len() - description_len - optional.len() - suffix.len();
    let mut header = prefix.to_vec();
    header.extend(std::iter::repeat_n(b'a', description_len));
    header.extend_from_slice(optional);
    header.extend(std::iter::repeat_n(b'b', optional_len));
    header.extend_from_slice(suffix);
    fs::write(skill_dir.join("SKILL.md"), header).unwrap();
    assert_eq!(
        discover_skills(&[root(&exact, SkillOrigin::User, "user")])
            .unwrap()
            .skills
            .len(),
        1
    );

    let too_large = TempTree::new();
    let skill_dir = too_large.path().join("large");
    fs::create_dir_all(&skill_dir).unwrap();
    let mut header = prefix.to_vec();
    header.extend(std::iter::repeat_n(b'a', description_len));
    header.extend_from_slice(optional);
    header.extend(std::iter::repeat_n(b'b', optional_len + 1));
    header.extend_from_slice(suffix);
    fs::write(skill_dir.join("SKILL.md"), header).unwrap();
    assert!(matches!(
        discover_skills(&[root(&too_large, SkillOrigin::User, "user")]),
        Err(SkillIndexError::HeaderTooLarge { .. })
    ));
}

#[test]
fn root_and_directory_entry_caps_are_exact_and_typed() {
    let root_limit = TempTree::new();
    for index in 0..256 {
        fs::write(
            root_limit.path().join(format!("ignored-{index}")),
            b"ignored",
        )
        .unwrap();
    }
    assert!(
        discover_skills(&[root(&root_limit, SkillOrigin::User, "user")])
            .unwrap()
            .skills
            .is_empty()
    );

    let over_limit = TempTree::new();
    for index in 0..257 {
        fs::write(
            over_limit.path().join(format!("ignored-{index}")),
            b"ignored",
        )
        .unwrap();
    }
    assert!(matches!(
        discover_skills(&[root(&over_limit, SkillOrigin::User, "user")]),
        Err(SkillIndexError::EntryLimit { .. })
    ));

    let first = TempTree::new();
    let second = TempTree::new();
    let third = TempTree::new();
    let roots = [
        root(&first, SkillOrigin::User, "user"),
        root(&second, SkillOrigin::Project, "project"),
        root(&third, SkillOrigin::Project, "other project"),
    ];
    assert!(matches!(
        discover_skills(&roots),
        Err(SkillIndexError::TooManyRoots)
    ));
}

#[test]
fn aggregate_header_limit_accepts_exact_total_and_rejects_one_more_header() {
    const HEADER_LIMIT: usize = 16 * 1024;
    fn write_max_header(tree: &TempTree, index: usize) {
        let directory_name = format!("skill-{index:03}");
        let directory = tree.path().join(&directory_name);
        fs::create_dir(&directory).unwrap();
        let prefix = format!("---\nname: {directory_name}\ndescription: ");
        let optional = "\nargument-hint: ";
        let suffix = "\n---\n";
        let optional_len = HEADER_LIMIT - prefix.len() - 8 * 1024 - optional.len() - suffix.len();
        let mut header = prefix.into_bytes();
        header.extend(std::iter::repeat_n(b'a', 8 * 1024));
        header.extend_from_slice(optional.as_bytes());
        header.extend(std::iter::repeat_n(b'b', optional_len));
        header.extend_from_slice(suffix.as_bytes());
        assert_eq!(header.len(), HEADER_LIMIT);
        fs::write(directory.join("SKILL.md"), header).unwrap();
    }

    let exact = TempTree::new();
    for index in 0..64 {
        write_max_header(&exact, index);
    }
    assert_eq!(
        discover_skills(&[root(&exact, SkillOrigin::User, "user")])
            .unwrap()
            .skills
            .len(),
        64
    );

    let too_large = TempTree::new();
    for index in 0..65 {
        write_max_header(&too_large, index);
    }
    assert!(matches!(
        discover_skills(&[root(&too_large, SkillOrigin::User, "user")]),
        Err(SkillIndexError::AggregateHeaderTooLarge { .. })
    ));
}
