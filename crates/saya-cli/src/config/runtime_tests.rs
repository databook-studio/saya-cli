//! Tests for the composed runtime: the investigations-root resolution and
//! the approval-mode vocabulary.

use super::*;

/// The pure resolver prefers the env override, then sits beside the
/// state database; a parentless state db falls back to a relative root.
#[test]
fn investigations_root_prefers_env_then_the_state_parent() {
    assert_eq!(
        investigations_root_from(
            Some(OsStr::new("/tmp/inv")),
            Path::new("/data/saya/state.sqlite3")
        ),
        PathBuf::from("/tmp/inv")
    );
    assert_eq!(
        investigations_root_from(None, Path::new("/data/saya/state.sqlite3")),
        PathBuf::from("/data/saya/investigations")
    );
    assert_eq!(
        investigations_root_from(None, Path::new("state.sqlite3")),
        PathBuf::from("investigations")
    );
}

/// The name map carries every mode the grammar parses, and each name
/// round-trips through the parser — the maps and `FromStr` cannot drift,
/// so a mode the flag accepts is a mode the status bar, the session
/// record, and `/approvals` can all name.
#[test]
fn the_approval_name_map_covers_the_whole_vocabulary() {
    for (value, name) in [
        ("ask", "ask"),
        ("read-only", "read-only"),
        ("never", "never"),
        ("bypass", "bypass"),
    ] {
        let options = GlobalOptions {
            approval_mode: Some(value.to_string()),
            ..Default::default()
        };
        assert_eq!(approval_name(&options).unwrap(), name);
        assert_eq!(
            approval_name(&options)
                .unwrap()
                .parse::<saya_agent::ApprovalPolicy>()
                .unwrap(),
            approval_mode(&options).unwrap(),
            "the name re-parses to the same mode: the vocabulary round-trips"
        );
    }
}
