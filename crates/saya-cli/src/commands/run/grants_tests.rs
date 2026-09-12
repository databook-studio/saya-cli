//! The resume-authority pair (the interpreter approval's design §4): the
//! grant a resume carries is derived from the journal's `PlanApproved`
//! payload — never from `spec.json`, which anything with the run directory's
//! permissions could rewrite between invocations.

use std::fs;
use std::path::PathBuf;

use saya_harness::journal::Journal;
use saya_types::{Capabilities, InterpreterScope, RunEvent};

use super::grants::journal_grants;

fn temp_dir(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-run-grants-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn journal_with(scopes: Vec<String>, dir: &PathBuf) {
    Journal::open(dir)
        .append(&RunEvent::PlanApproved { scopes })
        .expect("the approval is journaled");
}

fn spec_scopes(interpreters: &[&str]) -> Capabilities {
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    if !interpreters.is_empty() {
        capabilities.interpreter = Some(
            InterpreterScope::new(interpreters.iter().map(|p| (*p).to_owned()).collect())
                .expect("the spec's interpreters are refusal-list names"),
        );
    }
    capabilities
}

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
        .interpreter
        .expect("the journal granted the interpreter, so the resume re-grants it");
    assert_eq!(
        interpreters.programs,
        vec!["python3".to_owned()],
        "the resume re-grants exactly the journaled token, never the spec's extra name"
    );
    assert!(
        grant.workspace_write,
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
        grant.interpreter.is_none(),
        "a scope the journal never stated must not ride in from spec.json"
    );
    assert!(
        grant.workspace_write,
        "what the journal stated still re-grants"
    );

    let _ = fs::remove_dir_all(dir);
}

/// The pre-payload journal: a line written before the `scopes` field existed
/// carries no field and must replay — the run resumes, and the interpreter
/// family is granted nowhere. A spec.json carrying the token beside such a
/// journal is by definition post-hoc: the grammar did not exist when the
/// approval was journaled, so the file's token cannot be an approval.
#[test]
fn a_pre_payload_journal_resumes_without_an_interpreter_grant() {
    let dir = temp_dir("pre-payload");
    let legacy = "{\"type\":\"run_started\"}\n{\"type\":\"plan_approved\"}\n";
    fs::write(dir.join(saya_harness::journal::EVENTS_FILE), legacy).unwrap();

    let spec = spec_scopes(&["python3"]);
    let grant = journal_grants(&dir, &spec).expect("an old journal replays");
    assert!(
        grant.interpreter.is_none(),
        "a pre-payload journal grants no interpreter, whatever the spec says"
    );
    assert!(
        grant.workspace_write,
        "the spec fallback keeps the scopes an old journal could not state"
    );

    let _ = fs::remove_dir_all(dir);
}

/// A journal with no approval on record at all stands where the engine's
/// resume contract will refuse it: the derivation still returns the spec,
/// stripped, rather than granting anything the journal never mentioned.
#[test]
fn a_journal_without_an_approval_grants_nothing_the_spec_states_alone() {
    let dir = temp_dir("unapproved");
    Journal::open(&dir).append(&RunEvent::RunStarted).unwrap();

    let spec = spec_scopes(&["python3"]);
    let grant = journal_grants(&dir, &spec).expect("the derivation reads the journal");
    assert!(grant.interpreter.is_none());
    assert!(
        grant.workspace_write,
        "the unapproved refusal is the engine's"
    );

    let _ = fs::remove_dir_all(dir);
}
