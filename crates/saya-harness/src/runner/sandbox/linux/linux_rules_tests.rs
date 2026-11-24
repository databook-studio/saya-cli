//! The Landlock rule set's `/dev/null` contract, asserted without a
//! Landlock-capable kernel: `build_rules` is pure of syscalls, so these run
//! anywhere the Linux target compiles.

use super::*;

fn rules(handled_access_fs: u64) -> Vec<(std::ffi::CString, u64)> {
    let sb = RunSandbox::new([std::env::temp_dir()], Vec::<(String, u16)>::new())
        .expect("a real root constructs");
    build_rules(&sb, &[PathBuf::from("/bin")], handled_access_fs).expect("the rules marshal")
}

fn path_of(rule: &(std::ffi::CString, u64)) -> &str {
    rule.0.as_c_str().to_str().expect("paths are UTF-8")
}

/// The `/dev/null` rule is the POSIX background-stdin and redirection fix:
/// exactly one rule, on the file, granting read, write and truncate — and
/// only the rights the ruleset handles.
#[test]
fn the_dev_null_rule_grants_read_write_and_truncate_on_the_device() {
    let handled = ROOT_ALLOWED | EXEC_ALLOWED | sys::LANDLOCK_ACCESS_FS_REFER;
    let rules = rules(handled);
    let dev_null: Vec<_> = rules
        .iter()
        .filter(|rule| path_of(rule) == DEV_NULL)
        .collect();
    assert_eq!(dev_null.len(), 1, "exactly one /dev/null rule");
    assert_eq!(
        dev_null[0].1,
        sys::LANDLOCK_ACCESS_FS_READ_FILE
            | sys::LANDLOCK_ACCESS_FS_WRITE_FILE
            | sys::LANDLOCK_ACCESS_FS_TRUNCATE,
        "exactly read, write and truncate on the device"
    );
    assert_eq!(
        DEV_NULL_ALLOWED & handled,
        DEV_NULL_ALLOWED,
        "the device rights must be part of the handled set"
    );
}

/// No rule may name `/dev` or any other device under it: a directory grant
/// would make every device reachable, and only `/dev/null` is needed.
#[test]
fn no_rule_grants_the_dev_directory_or_another_device() {
    let handled = ROOT_ALLOWED | EXEC_ALLOWED | sys::LANDLOCK_ACCESS_FS_REFER;
    for rule in rules(handled) {
        let path = path_of(&rule);
        assert!(
            path != "/dev" && !(path.starts_with("/dev/") && path != DEV_NULL),
            "{path} must never be granted"
        );
    }
}

/// A kernel whose ABI predates TRUNCATE cannot handle it: a rule carrying an
/// unhandled right is EINVAL, so the device rule omits TRUNCATE and keeps
/// read+write.
#[test]
fn the_dev_null_rule_omits_truncate_when_the_ruleset_does_not_handle_it() {
    let handled = (ROOT_ALLOWED | EXEC_ALLOWED | sys::LANDLOCK_ACCESS_FS_REFER)
        & !sys::LANDLOCK_ACCESS_FS_TRUNCATE;
    let rules = rules(handled);
    let (_, allowed) = rules
        .iter()
        .find(|rule| path_of(rule) == DEV_NULL)
        .expect("/dev/null must carry a rule");
    assert_eq!(
        *allowed,
        sys::LANDLOCK_ACCESS_FS_READ_FILE | sys::LANDLOCK_ACCESS_FS_WRITE_FILE,
        "TRUNCATE is not handled by an older ABI and must be omitted"
    );
}
