use std::fs;

use super::{journal_grants, journal_with, spec_scopes, temp_dir};

/// (a) The grant survives a resume because it is derived from the journal's
/// `PlanApproved`: the journaled tokens parse back into exactly the
/// capabilities the original approval carried — the journaled `fetch:`
/// destination, and not the spec's extra one. When the journal states
/// scopes, the spec is not consulted at all. A word naming a removed door
/// (`interpreter:python3`, a removed door) is dropped rather than
/// refused: the journal is the durable record and cannot be edited, so a
/// resumed run carries what remains instead of failing to re-parse.
#[test]
fn the_resume_grant_is_derived_from_the_journal_s_plan_approved_payload() {
    let dir = temp_dir("journal-authority");
    journal_with(
        vec![
            "workspace-write".to_string(),
            "fetch:https+example.org".to_string(),
            "interpreter:python3".to_string(),
        ],
        &dir,
    );

    // The spec names a fetch the journal never approved — as a tampered or
    // careless file might — but the journal is the authority: the grant is
    // the journal's payload, parsed through the same grammar the approval was
    // typed in.
    let spec = spec_scopes(&["tampered.example"]);
    let grant = journal_grants(&dir, &spec).expect("the journal's scopes parse");
    assert!(
        grant.capabilities.workspace_write,
        "the journaled scopes re-grant in full"
    );
    let fetch = grant
        .capabilities
        .fetch
        .expect("the journaled fetch re-grants");
    assert_eq!(
        fetch.destinations.len(),
        1,
        "the resume re-grants exactly the journaled token"
    );
    assert_eq!(
        fetch.destinations[0].host, "example.org",
        "the journaled destination wins over the spec's"
    );
    assert!(
        !grant
            .tokens
            .iter()
            .any(|token| token.starts_with("interpreter:")),
        "a removed door is not a seed either: {:?}",
        grant.tokens
    );
    assert!(
        grant
            .tokens
            .iter()
            .any(|token| token == "fetch:https+example.org"),
        "the live words ride verbatim: {:?}",
        grant.tokens
    );

    let _ = fs::remove_dir_all(dir);
}

/// (b) The second half of the pair, and the point of it: a `spec.json`
/// edited on disk to add a fetch scope does not grant it. The journal states
/// the scopes, the journal lacks the token, so the resume carries no fetch
/// capability — the journal is the authority, not a file someone can edit
/// between invocations.
#[test]
fn a_spec_edited_on_disk_to_add_a_fetch_scope_does_not_grant_it() {
    let dir = temp_dir("spec-tampered");
    journal_with(vec!["workspace-write".to_string()], &dir);

    // The file now says the run approved a fetch destination; the journal
    // says it never did. The journal wins — no fetch grant.
    let spec = spec_scopes(&["tampered.example"]);
    let grant = journal_grants(&dir, &spec).expect("the journal's scopes parse");
    assert!(
        grant.capabilities.fetch.is_none(),
        "a scope the journal never stated must not ride in from spec.json"
    );
    assert!(
        grant.capabilities.workspace_write,
        "what the journal stated still re-grants"
    );

    let _ = fs::remove_dir_all(dir);
}
