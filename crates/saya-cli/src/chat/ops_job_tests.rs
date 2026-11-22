//! The job controls' one-release notes: one per spelling, said once a session.
//!
//! Row 171 of `ref/03-commands.md` folds six controls and `/follow-up` into
//! `/job <verb>`. They stay accepted at top level for one release, each with
//! one note — and the note is owed once, not once per use.

use super::{JobNotes, claim_job_note};
use crate::cli::CodingCommand;
use crate::slash::{JobAlias, JobRequest, SlashCommand, parse_slash_command};

/// One alias and the canonical line that is the same operation.
const ALIASES: &[(JobAlias, &str, &str)] = &[
    (JobAlias::Approve, "/approve job-a", "/job approve job-a"),
    (JobAlias::Deny, "/deny job-a", "/job deny job-a"),
    (
        JobAlias::Message,
        "/message job-a agent-a go",
        "/job message job-a agent-a go",
    ),
    (
        JobAlias::Pause,
        "/pause job-a agent-a",
        "/job pause job-a agent-a",
    ),
    (
        JobAlias::Continue,
        "/continue job-a agent-a",
        "/job continue job-a agent-a",
    ),
    (
        JobAlias::CancelChild,
        "/cancel-child job-a agent-a",
        "/job cancel-child job-a agent-a",
    ),
    (
        JobAlias::FollowUp,
        "/follow-up job-a agent-a --expected-completion-sequence 1 'go' --approve",
        "/job follow-up job-a agent-a --expected-completion-sequence 1 'go' --approve",
    ),
];

/// The command a line parses to, the spelling that stated it included.
fn request(line: &str) -> JobRequest {
    match parse_slash_command(line) {
        Ok(Some(SlashCommand::Coding(request))) => request,
        other => panic!("{line} parses to a job request: {other:?}"),
    }
}

/// Every top-level control is its `/job` verb: the same operation, one alias.
#[test]
fn every_control_is_its_job_verb_and_names_the_alias() {
    for (alias, alias_line, job_line) in ALIASES {
        let alias_request = request(alias_line);
        let job_request = request(job_line);
        assert_eq!(
            alias_request.command, job_request.command,
            "{alias_line} runs the same operation as {job_line}"
        );
        assert_eq!(
            alias_request.via_alias,
            Some(*alias),
            "{alias_line} records the spelling that stated it"
        );
        assert_eq!(
            job_request.via_alias, None,
            "{job_line} is the canonical spelling and owes no note"
        );
        assert_eq!(
            alias_request.note(),
            Some(alias.text()),
            "{alias_line} carries its one note"
        );
        assert!(
            alias.text().contains(&format!("/job {}", alias.name())),
            "the note names the replacement: {}",
            alias.text()
        );
        assert!(
            alias
                .text()
                .contains("this release is the last to answer it"),
            "the note names the release it is owed in: {}",
            alias.text()
        );
    }
}

/// The continuation spelling's note says which of the two `/continue` meanings
/// is the deprecated one.
#[test]
fn the_continue_control_note_names_the_bare_spelling() {
    let request = request("/continue job-a agent-a");
    assert_eq!(request.via_alias, Some(JobAlias::Continue));
    let note = request.note().expect("/continue <id> <agent> owes a note");
    assert!(
        note.contains("bare /continue continues a paused turn"),
        "the note points at the bare spelling: {note}"
    );
    assert_eq!(
        parse_slash_command("/continue").unwrap(),
        Some(SlashCommand::ContinueTurn),
        "bare /continue is the continuation turn, never a job control"
    );
}

/// A note is said once per session: the second use of the same spelling carries
/// none, and a different spelling still owes its own.
#[test]
fn a_note_is_said_once_per_session() {
    let mut notes = JobNotes::default();
    let mut first = request("/approve job-a");
    claim_job_note(&mut notes, &mut first);
    assert_eq!(
        first.note(),
        Some(JobAlias::Approve.text()),
        "the first use says the note"
    );

    let mut second = request("/approve job-b");
    claim_job_note(&mut notes, &mut second);
    assert_eq!(second.note(), None, "the second use says nothing");
    assert_eq!(
        second.command,
        CodingCommand::Approve { id: "job-b".into() },
        "the operation is unchanged: only the note is withheld"
    );

    let mut other = request("/deny job-a");
    claim_job_note(&mut notes, &mut other);
    assert_eq!(
        other.note(),
        Some(JobAlias::Deny.text()),
        "another spelling owes its own note"
    );
    assert_eq!(
        notes.said(),
        vec!["approve", "deny"],
        "the session records both"
    );
}

/// The canonical spellings never owe a note, however often they are used.
#[test]
fn the_job_verbs_owe_no_note() {
    let mut notes = JobNotes::default();
    for line in [
        "/job approve job-a",
        "/job continue job-a agent-a",
        "/start a task",
    ] {
        let mut request = request(line);
        claim_job_note(&mut notes, &mut request);
        assert_eq!(request.note(), None, "{line} owes no note");
    }
    assert!(notes.said().is_empty());
}
