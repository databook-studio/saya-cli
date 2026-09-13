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

/// The resume's seeds are the journal payload's own words — exactly what the
/// original approval stated, carried grant tokens (`sql:<connection>`)
/// included — so a resumed run's frozen decider consults what the journal
/// approved, never the spec. A spec.json edited between invocations cannot
/// add a seed: only the journal's payload states grant words.
#[test]
fn the_resume_s_seeds_are_the_journal_s_payload_words() {
    let dir = temp_dir("seed-words");
    journal_with(
        vec![
            "workspace-write".to_string(),
            "sql:analytics".to_string(),
            "sql:staging".to_string(),
        ],
        &dir,
    );
    // The spec carries neither sql token — as a tampered file might carry
    // anything — but the journal is the seed authority.
    let spec = spec_scopes(&[]);
    let grant = journal_grants(&dir, &spec).expect("the journal's scopes parse");
    assert_eq!(
        grant.tokens,
        vec![
            "workspace-write".to_owned(),
            "sql:analytics".to_owned(),
            "sql:staging".to_owned(),
        ],
        "the seeds are the journal's stated words, verbatim and in order"
    );
    assert!(
        grant.capabilities.workspace_write,
        "the capability words re-grant alongside the seeds"
    );

    let _ = fs::remove_dir_all(dir);
}

/// A carried token the journal states re-parses on the run surface (U4
/// wired `sql:` there), so the payload the fresh run recorded is exactly
/// the payload a resume replays — the words a run seeds with are the words
/// it journaled, round trip.
#[test]
fn a_carried_sql_token_in_a_journal_re_grants_as_a_seed() {
    let dir = temp_dir("carried-seed");
    journal_with(vec!["sql:analytics".to_string()], &dir);
    let spec = Capabilities::default();
    let grant = journal_grants(&dir, &spec).expect("the journal's carried token parses");
    assert_eq!(
        grant.tokens,
        vec!["sql:analytics".to_owned()],
        "the resumed decider is seeded with the connection's grant"
    );
    assert!(
        !grant.capabilities.workspace_write
            && !grant.capabilities.scratch
            && grant.capabilities.fetch.is_none()
            && grant.capabilities.runner.is_none()
            && grant.capabilities.interpreter.is_none()
            && grant.capabilities.endpoints.as_map().is_empty(),
        "a carried token is a grant word, not a plan capability"
    );

    let _ = fs::remove_dir_all(dir);
}
