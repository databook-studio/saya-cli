//! The old spellings keep working for one release: a `[tools] allow` entry
//! that names `run_command`, `http_fetch` or `http_download` narrows the
//! surface exactly as its successor's name does, and states a deprecation note
//! rather than an error.
//!
//! The successor table is the catalog's, so the list, an advertisement, and the
//! doctor's diagnosis cannot disagree about what an old spelling means.

use saya_config::{ResolvedTools, ToolsAllow, resolve};

use super::{with_cli_tools, with_user};

/// Every old spelling resolves to its successor's name, admits what the
/// successor admits, and says so — one note per stated spelling, naming both
/// words.
#[test]
fn every_old_spelling_maps_to_its_successor_with_one_note() {
    for (old, successor) in [
        ("run_command", "shell"),
        ("http_fetch", "web_fetch"),
        ("http_download", "web_fetch"),
    ] {
        let resolved = resolve(with_user(&format!(
            "[tools]\nallow = [\"{old}\", \"read\"]\n"
        )))
        .expect("an old spelling is not an error");
        assert!(
            resolved.tools.admits(successor),
            "`{old}` must keep advertising `{successor}`"
        );
        assert_eq!(
            resolved.tools.stated(),
            Some(vec!["read".to_string(), successor.to_string()]),
            "`{old}` is stored under its successor's name"
        );
        let notes = resolved.tools.deprecation_notes();
        assert_eq!(
            notes.len(),
            1,
            "one stated old spelling states one note: {notes:?}"
        );
        let note = &notes[0];
        assert!(
            note.contains(old) && note.contains(successor),
            "the note names the spelling and its successor: {note}"
        );
        assert!(
            note.contains("deprecated") && note.contains("one release"),
            "the note says what it is and for how long: {note}"
        );
    }
}

/// A canonical list states no note at all: the diagnosis is the old spelling's,
/// not the setting's.
#[test]
fn a_canonical_list_states_no_note() {
    let resolved = resolve(with_user(
        "[tools]\nallow = [\"shell\", \"web_fetch\", \"read\"]\n",
    ))
    .expect("a current list resolves");
    assert!(
        resolved.tools.deprecation_notes().is_empty(),
        "nothing deprecated was stated: {:?}",
        resolved.tools.deprecation_notes()
    );
}

/// The `--tools` route is the same list under the same rule: a stated old
/// spelling resolves to its successor and carries the note, so a launch flag
/// and a config layer cannot mean different things.
#[test]
fn the_cli_route_maps_the_old_spellings_the_same_way() {
    let resolved = resolve(with_cli_tools(&["run_command"])).expect("the flag's list resolves");
    assert!(resolved.tools.admits("shell"), "`run_command` keeps shell");
    assert_eq!(
        resolved.tools.deprecation_notes().len(),
        1,
        "the flag's list states the same note: {:?}",
        resolved.tools.deprecation_notes()
    );
    let stated =
        ResolvedTools::from_stated(Some(&["run_command".to_string()])).expect("a valid list");
    assert!(stated.admits("shell"));
    assert_eq!(stated.deprecation_notes().len(), 1);
}

/// The old spellings are still governed names — a list that states one is not
/// refused as unknown — and they are still not connection-tied tools.
#[test]
fn the_old_spellings_stay_governed_names() {
    for old in ["run_command", "http_fetch", "http_download"] {
        assert!(
            saya_config::is_governed_tool(old),
            "`{old}` must stay a name the list may state"
        );
        assert!(
            !saya_config::is_database_tool(old),
            "`{old}` is a capability, not a connection-tied tool"
        );
        assert!(
            ToolsAllow::parse([old]).is_ok(),
            "`{old}` parses, never an error"
        );
    }
}
