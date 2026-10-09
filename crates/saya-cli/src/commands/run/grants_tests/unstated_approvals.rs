use std::fs;

use saya_harness::journal::Journal;
use saya_types::{Capabilities, InterpreterScope, RunEvent};

use super::{journal_grants, spec_scopes, temp_dir};

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
        grant.capabilities.interpreter.is_none(),
        "a pre-payload journal grants no interpreter, whatever the spec says"
    );
    assert!(
        grant.capabilities.workspace_write,
        "the spec fallback keeps the scopes an old journal could not state"
    );
    assert!(
        grant.tokens.is_empty(),
        "the spec fallback states no seed words: only the journal's own scopes seed \
         the resumed decider"
    );

    let _ = fs::remove_dir_all(dir);
}

/// A `--allow none` run's journal states the empty approval — the exact
/// line today's drive writes (`"scopes":[]`) — and a resume re-grants
/// exactly that: nothing. A `spec.json` edited between invocations to add
/// `workspace-write`, `scratch`, `fetch`, `runner` or `interpreter` grants
/// none of them: the journal stated the empty set, and the grant is the
/// journal's, never the file's. The pair with
/// [`a_pre_payload_journal_resumes_without_an_interpreter_grant`]: the
/// field-less line is "scopes unstated" (the spec stands in), the empty
/// payload is "the empty approval was stated" (nothing rides in) — the two
/// must never fold back into one case.
#[test]
fn a_none_run_s_resume_grants_nothing_an_edited_spec_states() {
    let dir = temp_dir("none-run");
    let none_run = "{\"type\":\"run_started\"}\n{\"type\":\"plan_approved\",\"scopes\":[]}\n";
    fs::write(dir.join(saya_harness::journal::EVENTS_FILE), none_run).unwrap();

    // The spec now carries every capability an edited file could add; the
    // journal stated the empty approval, so the resume grants none of them.
    let mut spec = Capabilities::default();
    spec.workspace_write = true;
    spec.scratch = true;
    spec.fetch = Some(
        saya_types::FetchScope::new(vec![
            saya_types::Destination::new("https", "example.com")
                .expect("the spec's destination is shape-valid"),
        ])
        .expect("the spec's fetch scope parses"),
    );
    spec.runner = Some(
        saya_types::RunnerScope::new(vec!["bench".to_owned()])
            .expect("the spec's runner is a bare name"),
    );
    spec.interpreter = Some(
        InterpreterScope::new(vec!["python3".to_owned()])
            .expect("the spec's interpreter is a refusal-list name"),
    );
    let grant = journal_grants(&dir, &spec).expect("the none run's journal replays");
    assert!(
        !grant.capabilities.workspace_write
            && !grant.capabilities.scratch
            && grant.capabilities.fetch.is_none()
            && grant.capabilities.runner.is_none()
            && grant.capabilities.interpreter.is_none()
            && grant.capabilities.endpoints.as_map().is_empty(),
        "a none run's resume grants nothing an edited spec.json states: {:?}",
        grant.capabilities
    );
    assert!(
        grant.tokens.is_empty(),
        "the empty approval seeds no grant words either: {:?}",
        grant.tokens
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
    assert!(grant.capabilities.interpreter.is_none());
    assert!(
        grant.capabilities.workspace_write,
        "the unapproved refusal is the engine's"
    );

    let _ = fs::remove_dir_all(dir);
}
