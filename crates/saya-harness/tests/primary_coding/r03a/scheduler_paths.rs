//! Public path-lease semantics include component boundaries and conservative Unicode.
use saya_harness::coding::integration::AssignedPaths;
#[test]
fn path_overlap_respects_components_and_case_without_raw_prefix_false_negatives() {
    for (left, right, expected) in [
        (vec!["src"], vec!["src-foo", "src/z"], true),
        (vec!["src/a"], vec!["src/ab"], false),
        (vec!["SRC"], vec!["src/nested"], true),
        (vec!["source"], vec!["docs"], false),
        (vec!["café"], vec!["docs"], true),
    ] {
        let left = AssignedPaths::parse(left.into_iter().map(str::to_owned)).unwrap();
        let right = AssignedPaths::parse(right.into_iter().map(str::to_owned)).unwrap();
        assert_eq!(left.overlaps(&right), expected);
        assert_eq!(right.overlaps(&left), expected);
    }
}
