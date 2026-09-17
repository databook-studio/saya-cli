//! H1b green — the session deny list, written before any deny seam exists.
//!
//! Every test below named an API that did not exist when written
//! (`session_deny::*`, the `HostLaunch` deny readers, the config
//! `[session_commands] deny` key, the `--deny` flag, the journal deny
//! kinds). The red compile failure is pasted verbatim into `REPORT.md`; the
//! seams below are the green implementation those tests pinned.

use crate::interactive::session_deny;

/// 1. A denied name refuses at every door before prompt, grant, and bypass
///    (`run_command`, `run_program`, the interpreter door — deny first,
///    then grant lookup, then the approval prompt, then bypass auto-allow).
#[test]
fn a_denied_name_refuses_at_every_door_before_prompt_grant_and_bypass() {
    let deny =
        session_deny::SessionDeny::from_names(["curl".to_owned()]).expect("a bare name builds");
    for door in ["run_command", "run_program", "interpreter"] {
        assert!(
            deny.contains("curl"),
            "a denied name refuses at the {door} door"
        );
    }
    assert!(
        !deny.contains("make"),
        "a name outside the list is not refused"
    );
    // The refusal's bytes name the policy and the not-bounded clause.
    let refusal = session_deny::denied_refusal("curl");
    assert!(
        refusal.contains("deny list") && refusal.contains("allowed programs may still invoke it"),
        "the refusal carries the policy and the not-bounded clause: {refusal}"
    );
}

/// 2. `--deny` alone composes nothing and gates the contained doors
///    (refusal-only; gates the doors every session has, lane off included).
#[test]
fn the_deny_flag_composes_nothing_and_gates_the_contained_doors() {
    let launch =
        crate::interactive::session_host::HostLaunch::from_deny_for_tests(vec!["curl".to_owned()]);
    assert!(
        !launch.composes_lane(),
        "--deny alone must not compose the host lane"
    );
    assert_eq!(
        launch.deny_list(),
        vec!["curl".to_owned()],
        "the launch still carries the deny list"
    );
}

/// 3. Runs refuse the deny flag with pinned wording
///    (deny is session-shaped; a run programs are pre-declared scopes).
#[test]
fn runs_refuse_the_deny_flag() {
    let error = crate::commands::run::scopes::refuse_deny_on_run_for_tests();
    assert!(
        error.contains("deny is session-shaped"),
        "the run refusal says what deny is: {error}"
    );
}

/// 4. Path-shaped and glob deny entries refuse at launch
///    (a deny entry is a bare name, never a path or a pattern).
#[test]
fn path_shaped_and_glob_deny_entries_refuse_at_launch() {
    for entry in ["bin/curl", "../curl", "cargo test", "cur*"] {
        assert!(
            session_deny::validate_deny_entry(entry).is_err(),
            "deny entry {entry:?} must refuse at launch"
        );
    }
    assert!(
        session_deny::validate_deny_entry("curl").is_ok(),
        "a bare name is a valid deny entry"
    );
}

/// 5. Launch granting and denying the same name exits 2
///    (a statement that grants what it refuses).
#[test]
fn launch_granting_and_denying_the_same_name_exits_2() {
    let error = session_deny::launch_contradiction("curl");
    assert!(
        error.contains("cannot both grant and deny"),
        "the contradiction names itself: {error}"
    );
}

/// 6. The project layer cannot deny either: a typed resolve error
///    (the H1a trust boundary for `[host_commands]`; the project file
///    is model-writable once `workspace_write` is granted).
#[test]
fn the_project_layer_cannot_deny_either() {
    let input = saya_config::ResolutionInput::new(saya_config::ConnectionsFile::default())
        .with_project(
            saya_config::ConfigFile::from_toml("[session_commands]\ndeny = ['curl']\n")
                .expect("fixture parses"),
        );
    let error = match saya_config::resolve(input) {
        Err(error) => error.to_string(),
        Ok(_) => panic!("a project-layer [session_commands] must be a typed resolve error"),
    };
    assert!(
        error.contains("session_commands") && error.contains("project"),
        "the refusal names the section and the layer: {error}"
    );
}

/// 7. The refusal carries the pinned bytes and the not-bounded clause
///    (the policy and where stated; the not-a-failure disclaimer;
///    the list bounds only the named program).
#[test]
fn the_denied_refusal_carries_the_pinned_bytes_and_the_not_bounded_clause() {
    let refusal = session_deny::denied_refusal("curl");
    for pin in [
        "deny list",
        "stated at launch or in user config",
        "this is saya's refusal, not a program failure",
        "bounds only the program named in the ask",
        "allowed programs may still invoke it",
    ] {
        assert!(
            refusal.contains(pin),
            "the refusal carries the pinned bytes ({pin}): {refusal}"
        );
    }
}

/// H3 reproduction (documents the confirmed limit, must fail before the
/// fix): a deny entry refuses its own exact spelling only — a renamed copy
/// (`mycurl` for denied `curl`) is outside the list. Until content-based
/// detection exists (explicitly not this product), the refusal text must say
/// so rather than read as an execution block.
#[test]
fn the_deny_refusal_states_the_exact_name_limit() {
    let deny =
        session_deny::SessionDeny::from_names(["curl".to_owned()]).expect("a bare name builds");
    assert!(
        !deny.contains("mycurl"),
        "deny is exact-name: a renamed copy is outside the list"
    );
    let refusal = session_deny::denied_refusal("curl");
    assert!(
        refusal.contains("exact")
            && refusal.contains("name")
            && (refusal.contains("renamed") || refusal.contains("spelling")),
        "the refusal must state the exact-name limit rather than overstate what deny bounds: {refusal}"
    );
}

/// 8. The deny journal events carry shape, ordering, and redaction
///    (`session-deny-list` once at start when non-empty;
///    `session-command-denied` per firing; redacted; before refusal).
#[test]
fn the_deny_journal_events_carry_shape_ordering_and_redaction() {
    use saya_store::{JournalEvent, SessionJournal};
    let dir = std::env::temp_dir().join(format!("saya-deny-red-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("state dir creates");
    let journal = SessionJournal::open(&dir);
    journal
        .deny_list(&["curl".to_owned()])
        .expect("the start event journals");
    journal
        .command_denied("curl", &["-s".to_owned()], "run_command")
        .expect("the firing journals");
    journal
        .deny_list(&[])
        .expect("an empty list journals nothing");
    let events = journal.read().expect("the journal reads");
    assert_eq!(
        events,
        vec![
            JournalEvent::DenyList {
                programs: vec!["curl".to_owned()],
            },
            JournalEvent::CommandDenied {
                program: "curl".to_owned(),
                argv: vec!["-s".to_owned()],
                door: "run_command".to_owned(),
            },
        ],
        "shape and ordering: the list once at start, then the firing"
    );
    let raw = std::fs::read_to_string(dir.join("journal.ndjson")).expect("the journal reads");
    assert!(
        raw.contains("session-deny-list") && raw.contains("session-command-denied"),
        "both kinds write their event words: {raw}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
