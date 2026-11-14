//! In-place deletion policy: the job's own root scratch tree, any Git
//! metadata component, and credential-shaped paths are the only paths deletion
//! must refuse.

use super::omitted_path;

#[test]
fn in_place_delete_refuses_policy_paths() {
    // The root scratch tree, a nested repository (file or directory), and a
    // credential-shaped path are refused.
    for path in [
        ".saya",
        ".saya/inplace/home",
        ".git",
        "nested/.git",
        "nested/.git/config",
        ".env",
        "config/id_rsa",
    ] {
        assert!(omitted_path(path), "{path} must not be deleted");
    }
    // A sibling that merely shares a prefix, and ordinary files, are allowed.
    for path in [
        "report.md",
        "nested",
        "nested/report.md",
        ".envrc",
        "gitignore",
    ] {
        assert!(!omitted_path(path), "{path} is not omitted");
    }
}
