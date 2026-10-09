use std::fs;

use super::{journal_grants, journal_with, spec_scopes, temp_dir};

/// (a) The grant survives a resume because it is derived from the journal's
/// `PlanApproved`: the journaled tokens parse back into exactly the
/// capabilities the original approval carried — `interpreter:python3`, and
/// not `interpreter:node` even when the spec names it too. When the journal
/// states scopes, the spec is not consulted at all.
#[test]
fn the_resume_grant_is_derived_from_the_journal_s_plan_approved_payload() {
    let dir = temp_dir("journal-authority");
    journal_with(
        vec![
            "workspace-write".to_string(),
            "interpreter:python3".to_string(),
        ],
        &dir,
    );

    // The spec carries both interpreters — as a tampered or careless file
    // might — but the journal is the authority: the grant is the journal's
    // payload, parsed through the same grammar the approval was typed in.
    let spec = spec_scopes(&["node", "python3"]);
    let grant = journal_grants(&dir, &spec).expect("the journal's scopes parse");
    let interpreters = grant
        .capabilities
        .interpreter
        .expect("the journal granted the interpreter, so the resume re-grants it");
    assert_eq!(
        interpreters.programs,
        vec!["python3".to_owned()],
        "the resume re-grants exactly the journaled token, never the spec's extra name"
    );
    assert!(
        grant.capabilities.workspace_write,
        "the journaled scopes re-grant in full"
    );

    let _ = fs::remove_dir_all(dir);
}

/// (b) The second half of the pair, and the point of it: a `spec.json`
/// edited on disk to add the interpreter scope does not grant it. The
/// journal states the scopes, the journal lacks the token, so the resume
/// carries no interpreter capability — the journal is the authority, not a
/// file someone can edit between invocations.
#[test]
fn a_spec_edited_on_disk_to_add_the_scope_does_not_grant_it() {
    let dir = temp_dir("spec-tampered");
    journal_with(vec!["workspace-write".to_string()], &dir);

    // The file now says the run approved an interpreter; the journal says
    // it never did. The journal wins — no interpreter grant.
    let spec = spec_scopes(&["python3"]);
    let grant = journal_grants(&dir, &spec).expect("the journal's scopes parse");
    assert!(
        grant.capabilities.interpreter.is_none(),
        "a scope the journal never stated must not ride in from spec.json"
    );
    assert!(
        grant.capabilities.workspace_write,
        "what the journal stated still re-grants"
    );

    let _ = fs::remove_dir_all(dir);
}
