use super::*;

/// The five deprecated spellings each state one note, and the canonical
/// spellings state none. A note is said once per command: it rides the request
/// the shared run seam prints, never the parser twice.
#[test]
fn every_deprecated_spelling_states_one_note() {
    let deprecated = [
        ("contract", ""),
        ("queue", ""),
        ("confirm", "ki-a86a3f"),
        ("reject", "ki-a86a3f"),
        ("approve-all", "--yes"),
    ];
    for (spelling, arg) in deprecated {
        let request = parse_contract_command(spelling, arg).unwrap().unwrap();
        assert!(
            request.note.is_some(),
            "/{spelling} is a deprecated spelling: it must say so"
        );
    }
    let canonical = [
        ("contracts", ""),
        ("contracts", "list"),
        ("contracts", "queue"),
        ("contracts", "confirm ki-a86a3f"),
        ("contracts", "reject ki-a86a3f"),
        ("contracts", "confirm-all --yes"),
        ("remember", "analytics.public.orders alias orders"),
        ("forget", "ki-a86a3f"),
    ];
    for (spelling, arg) in canonical {
        let request = parse_contract_command(spelling, arg).unwrap().unwrap();
        assert_eq!(request.note, None, "/{spelling} {arg} is canonical");
    }
}

/// Every note names the spelling that replaces it, and is one line — the
/// surface prints it once, so a two-line note would read as two answers.
#[test]
fn the_notes_name_the_new_spelling() {
    for (spelling, arg, replacement) in [
        ("contract", "", "/contracts"),
        ("queue", "", "/contracts queue"),
        ("confirm", "ki-a86a3f", "/contracts confirm"),
        ("reject", "ki-a86a3f", "/contracts reject"),
        ("approve-all", "--yes", "/contracts confirm-all"),
    ] {
        let note = parse_contract_command(spelling, arg)
            .unwrap()
            .unwrap()
            .note
            .expect("a deprecated spelling states its note");
        assert!(
            note.contains(replacement),
            "/{spelling} note must name {replacement}: {note}"
        );
        assert!(
            note.contains(spelling),
            "/{spelling} note must name the spelling it deprecates: {note}"
        );
        assert!(
            note.contains("deprecated") && note.contains("next release"),
            "/{spelling} note must say it is deprecated and for how long: {note}"
        );
        assert!(!note.contains('\n'), "/{spelling} note is one line: {note}");
    }
}

/// `/approve-all` reads like a job approval, and never was one: its note says
/// the command is about knowledge claims and points at the job command.
#[test]
fn the_approve_all_note_is_about_knowledge_claims_not_job_approvals() {
    let note = parse_contract_command("approve-all", "--yes")
        .unwrap()
        .unwrap()
        .note
        .expect("the deprecated spelling states its note");
    assert!(note.contains("/contracts confirm-all"), "{note}");
    assert!(note.contains("knowledge claims"), "{note}");
    assert!(
        note.contains("never job approvals"),
        "the note must say it is not about job approvals: {note}"
    );
    assert!(
        note.contains("/approve <id>"),
        "the note must point at the job command: {note}"
    );
}
