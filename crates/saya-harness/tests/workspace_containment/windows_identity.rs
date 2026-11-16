use super::*;
use std::io;

/// A file another writer renames over while a handle holds it answers from
/// NTFS's deleted-files area for an instant. That name is not an escape — the
/// file the walk validated was replaced — so the containment check must
/// classify it as a replacement, never as a path outside the root.
#[test]
fn the_deleted_files_area_is_a_replacement_not_an_escape() {
    let deleted = Path::new(r"\\?\C:\$Extend\$Deleted\000500000000B31853FE9168");
    assert!(
        saya_harness::workspace::windows_identity::is_deleted_file_area(deleted),
        "the volume's deleted-files area is the transient name of a replaced file"
    );
    // A tree that merely uses those names deeper down is not the volume's
    // area: only the volume root's `$Extend\$Deleted` is the kernel's.
    let nested = Path::new(r"\\?\C:\workspace\sub\$Extend\$Deleted\file");
    assert!(
        !saya_harness::workspace::windows_identity::is_deleted_file_area(nested),
        "a nested look-alike must stay an ordinary path"
    );
    let outside = Path::new(r"\\?\C:\Users\other\file.txt");
    assert!(
        !saya_harness::workspace::windows_identity::is_deleted_file_area(outside),
        "an ordinary outside path must stay outside"
    );
}

/// The refusals a concurrent replace gives while the old file is being
/// deleted are told apart from a refusal that will not settle.
#[test]
fn transient_replacement_errors_are_classified() {
    for code in [2, 3, 5, 32, 303] {
        assert!(
            saya_harness::workspace::windows_identity::is_transient_replacement_error(
                &io::Error::from_raw_os_error(code)
            ),
            "os error {code} is the concurrent-replace window"
        );
    }
    assert!(
        !saya_harness::workspace::windows_identity::is_transient_replacement_error(
            &io::Error::from_raw_os_error(1234)
        ),
        "an unrelated error must keep its own meaning"
    );
}

/// The verdict the containment walk applies to a resolved anchor: a transient
/// refusal and NTFS's deleted-files area are replacements, an ordinary path
/// outside the root is still an escape. Removing either replacement branch
/// turns this red.
#[test]
fn a_replace_during_resolution_is_never_an_escape() {
    use saya_harness::workspace::contain::canonical_verdict;
    let root = Path::new(r"\\?\C:\ws");
    let anchor = Path::new(r"\\?\C:\ws\shared.txt");
    for code in [2, 3, 5, 32, 303] {
        let verdict = canonical_verdict(
            root,
            anchor,
            Err(io::Error::from_raw_os_error(code)),
            "shared.txt",
            true,
        );
        assert!(
            matches!(&verdict, Err(HarnessError::IdentityChanged { .. })),
            "os error {code} during a concurrent replace must refuse as IdentityChanged, got {verdict:?}"
        );
    }
    let deleted = PathBuf::from(r"\\?\C:\$Extend\$Deleted\000500000000B31853FE9168");
    assert!(
        matches!(
            canonical_verdict(root, anchor, Ok(deleted), "shared.txt", true),
            Err(HarnessError::IdentityChanged { .. })
        ),
        "the deleted-files area must refuse as IdentityChanged"
    );
    let outside = PathBuf::from(r"\\?\C:\Users\other\file.txt");
    assert!(
        matches!(
            canonical_verdict(root, anchor, Ok(outside), "shared.txt", true),
            Err(HarnessError::PathOutsideRoot { .. })
        ),
        "a real outside path must stay an escape"
    );
}

/// The Windows containment identity is the volume serial plus the kernel's
/// file index from an open handle. Two different directories must be
/// distinguishable, or no post-open identity check could ever catch a swap.
#[test]
fn distinct_directories_have_distinct_identities() {
    let outer = std::env::temp_dir().join(format!("saya-ws-ident-{}", std::process::id()));
    let _ = fs::remove_dir_all(&outer);
    let first = outer.join("first");
    let second = outer.join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    let first_identity =
        saya_harness::workspace::windows_identity::directory_identity(&first).unwrap();
    let second_identity =
        saya_harness::workspace::windows_identity::directory_identity(&second).unwrap();
    assert_ne!(
        first_identity, second_identity,
        "the file index must distinguish two directories"
    );
    // A rename preserves the object's identity: the same directory under a
    // new name is still the same object, which is what a post-rename
    // verification relies on.
    let moved = outer.join("moved");
    fs::rename(&first, &moved).unwrap();
    assert_eq!(
        saya_harness::workspace::windows_identity::directory_identity(&moved).unwrap(),
        first_identity,
        "a rename must preserve the object's identity"
    );
    let _ = fs::remove_dir_all(&outer);
}
