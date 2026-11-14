//! The write-scope boundary is a property, not a list of cases: a safe
//! relative path is admitted, every traversal spelling is refused or outside
//! the scope, and `.git` metadata is never a writable path.

use proptest::prelude::*;
use saya_harness::coding::integration::AssignedPaths;
use saya_types::WorkspacePath;

fn safe_component() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9._-]{1,20}".prop_filter("a component, not a traversal or Git metadata", |part| {
        part != "." && part != ".." && !part.eq_ignore_ascii_case(".git")
    })
}

fn safe_path() -> impl Strategy<Value = String> {
    prop::collection::vec(safe_component(), 1..4).prop_map(|parts| parts.join("/"))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn path_traversal_property(safe in safe_path()) {
        let workspace = WorkspacePath::try_from(safe.clone())
            .expect("a safe relative path is admitted");
        prop_assert!(workspace.covers(&workspace.clone()));

        let scope = AssignedPaths::parse(vec![safe.clone()])
            .expect("a safe path is a valid scope");
        prop_assert!(scope.contains(&safe));
        let child = format!("{safe}/child");
        prop_assert!(scope.contains(&child), "a scope covers its descendants");
        let sibling = format!("{safe}x");
        prop_assert!(
            !scope.contains(&sibling),
            "a shared prefix is not a path boundary"
        );

        for evil in [
            format!("../{safe}"),
            format!("{safe}/../../escape"),
            format!("/{safe}"),
            format!("{safe}\\escape"),
            format!("C:/{safe}"),
            format!("C:\\{safe}"),
            format!("{safe}//child"),
            format!("{safe}/./child"),
            format!("{safe}/.git/config"),
            format!("{safe}/.GIT/config"),
            format!("{safe}\0"),
            String::new(),
            ".".to_owned(),
            "..".to_owned(),
        ] {
            prop_assert!(
                WorkspacePath::try_from(evil.clone()).is_err(),
                "{:?} is refused as a workspace path",
                evil
            );
            let parsed = AssignedPaths::parse(vec![evil.clone()]);
            prop_assert!(
                parsed.as_ref().map_or(true, |scope| !scope.contains(&evil)),
                "{:?} is outside every write scope",
                evil
            );
        }

        let whole = AssignedPaths::whole_checkout();
        prop_assert!(whole.contains(&safe));
        let git_path = format!("{safe}/.git/config");
        prop_assert!(!whole.contains(&git_path));
    }
}
