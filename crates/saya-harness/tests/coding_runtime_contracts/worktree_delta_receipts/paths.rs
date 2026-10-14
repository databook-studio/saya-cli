use saya_harness::coding::integration::{
    AssignedPaths, AssignedPathsError, MAX_ASSIGNED_PATH_BYTES, MAX_ASSIGNED_PATHS,
    casefold_collision,
};

fn owned(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|p| (*p).to_string()).collect()
}

#[test]
fn assigned_paths_reject_escaping_and_oversize_entries() {
    for bad in [
        "/abs",
        "../x",
        "a/../b",
        "a/./b",
        "a\\b",
        "",
        "a//b",
        ".git/config",
        "a/.GIT/x",
        "C:x",
        "a\0b",
    ] {
        assert_eq!(
            AssignedPaths::parse(owned(&[bad])),
            Err(AssignedPathsError::UnsafePath),
            "expected UnsafePath for {bad:?}"
        );
    }

    let oversize = "x".repeat(MAX_ASSIGNED_PATH_BYTES + 1);
    assert_eq!(
        AssignedPaths::parse(owned(&[&oversize])),
        Err(AssignedPathsError::TooLarge)
    );

    let entries: Vec<String> = (0..66)
        .map(|i| format!("{i:03}{}", "x".repeat(497)))
        .collect();
    assert_eq!(entries.len(), 66);
    assert_eq!(entries[0].len(), 500);
    assert_eq!(
        AssignedPaths::parse(entries),
        Err(AssignedPathsError::TooLarge)
    );

    let many: Vec<String> = (0..=MAX_ASSIGNED_PATHS).map(|i| format!("p{i}")).collect();
    assert_eq!(AssignedPaths::parse(many), Err(AssignedPathsError::TooMany));

    assert_eq!(
        AssignedPaths::parse(Vec::<String>::new()),
        Err(AssignedPathsError::Empty)
    );

    assert_eq!(
        AssignedPaths::parse(owned(&["a", "a"])),
        Err(AssignedPathsError::Duplicate)
    );
    assert_eq!(
        AssignedPaths::parse(owned(&["a/B", "a/b"])),
        Err(AssignedPathsError::Duplicate)
    );

    let parsed = AssignedPaths::parse(owned(&["src/lib", "README.md"])).expect("valid set parses");
    assert_eq!(parsed.len(), 2);
    assert!(!parsed.is_empty());
}

#[test]
fn prefix_membership_matches_component_boundary() {
    let assigned =
        AssignedPaths::parse(owned(&["src/lib", "README.md"])).expect("valid set parses");
    for contained in ["src/lib", "src/lib/a.rs", "README.md"] {
        assert!(
            assigned.contains(contained),
            "expected containment: {contained}"
        );
    }
    for absent in [
        "src/library",
        "src",
        "SRC/lib",
        "src/lib2/x",
        "../src/lib",
        "",
    ] {
        assert!(
            !assigned.contains(absent),
            "expected no containment: {absent}"
        );
    }

    let nfc = AssignedPaths::parse(owned(&["caf\u{e9}"])).expect("valid set parses");
    assert!(!nfc.contains("cafe\u{301}"));
}

#[test]
fn casefold_collision_detects_distinct_paths_only() {
    assert!(casefold_collision(["a/B", "a/b"]));
    assert!(casefold_collision(["\u{c9}x", "\u{e9}x"]));
    assert!(!casefold_collision(["a/b", "a/b"]));
    assert!(!casefold_collision(["a/b", "a/c"]));
    assert!(!casefold_collision(Vec::<&str>::new()));
}

#[test]
fn assigned_paths_debug_and_errors_carry_no_paths() {
    const SECRET: &str = "secret-dir/x";

    let parsed = AssignedPaths::parse(owned(&[SECRET])).expect("valid set parses");
    assert!(!format!("{parsed:?}").contains(SECRET));

    let oversize = "x".repeat(MAX_ASSIGNED_PATH_BYTES + 1);
    let errors = [
        AssignedPaths::parse(Vec::<String>::new()).unwrap_err(),
        AssignedPaths::parse((0..=MAX_ASSIGNED_PATHS).map(|i| format!("p{i}"))).unwrap_err(),
        AssignedPaths::parse(owned(&[&oversize])).unwrap_err(),
        AssignedPaths::parse(owned(&["/abs"])).unwrap_err(),
        AssignedPaths::parse(owned(&[SECRET, SECRET])).unwrap_err(),
    ];
    for error in errors {
        let display = format!("{error}");
        assert!(
            !display.contains(SECRET),
            "Display leaked a path: {display:?}"
        );
    }
}
