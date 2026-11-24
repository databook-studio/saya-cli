//! The consent line, as the operator reads it: the job, the task, the paths it
//! may write, one row per database identity, how to answer it — and the exact
//! proposal last.

use serde_json::{Value, json};

use super::text;

/// The identities two profiles resolve to: opaque, fixed and long enough that a
/// row's width is worth stating.
const ANALYTICS: &str = "p-0000000000000000000000000000000000000000000000000000000000000001";
const WAREHOUSE: &str = "p-0000000000000000000000000000000000000000000000000000000000000002";

/// A proposal shaped as today's builder writes one for an in-place job: the
/// scope it would edit, the two profiles it may read, and the plan it holds.
fn proposal() -> Value {
    json!({
        "baseline": {"kind": "edit_time", "plan_sha256": "0".repeat(64)},
        "budget_mode": "default",
        "database_origin": {"path": "/home/saya/connections.toml", "project_provided": false},
        "database_profiles": {"analytics": ANALYTICS, "warehouse": WAREHOUSE},
        "job_id": "pty-job",
        "mode": "in_place",
        "no_git_history": true,
        "operation": "start",
        "repository_id": "repo-a",
        "repository_path": "/work/repo",
        "revocation_epoch": 1,
        "snapshot_id": "pty-job",
        "task": "controlled task with helpers",
        "write_scope": ["a.txt", "b.txt"],
    })
}

#[test]
fn the_consent_states_the_job_its_task_and_the_paths_it_may_write() {
    let rows: Vec<String> = text("pty-job", &proposal())
        .split('\n')
        .map(str::to_owned)
        .collect();
    assert_eq!(
        rows[0], "Approve job pty-job with this exact scope:",
        "the job's own id opens the consent"
    );
    assert_eq!(rows[1], "task: controlled task with helpers");
    assert_eq!(
        rows[2], "write: a.txt, b.txt",
        "every path the job may write, in the order the start stated them"
    );
}

#[test]
fn every_identity_the_proposal_resolved_is_named_verbatim() {
    let rows: Vec<String> = text("pty-job", &proposal())
        .split('\n')
        .map(str::to_owned)
        .collect();
    assert!(
        rows.contains(&format!("\"analytics\" {ANALYTICS}")),
        "the first profile's own name and identity: {rows:?}"
    );
    assert!(
        rows.contains(&format!("\"warehouse\" {WAREHOUSE}")),
        "and the second's: {rows:?}"
    );
    // One row per identity: nothing is folded onto a row of its own.
    let identities = rows.iter().filter(|row| row.starts_with('"')).count();
    assert_eq!(identities, 2, "one row per database the job may read");
}

#[test]
fn the_consent_says_how_to_answer_it_and_keeps_the_proposal_verbatim() {
    let rows: Vec<String> = text("pty-job", &proposal())
        .split('\n')
        .map(str::to_owned)
        .collect();
    let answer = rows
        .iter()
        .position(|row| row == "Use `/job approve pty-job` or `/job deny pty-job`.")
        .expect("the consent names the keys that answer it");
    assert_eq!(
        rows.last().map(String::as_str),
        Some(proposal().to_string().as_str()),
        "the exact proposal follows the answer line, byte for byte"
    );
    assert_eq!(answer, rows.len() - 2, "and it is the row before it");
}

#[test]
fn a_job_that_may_write_nothing_states_no_write_row() {
    // A Git job edits its own checkout: its proposal states no write scope, and
    // the consent invents none.
    let mut arguments = proposal();
    arguments.as_object_mut().unwrap().remove("write_scope");
    let rows: Vec<String> = text("git-job", &arguments)
        .split('\n')
        .map(str::to_owned)
        .collect();
    assert!(
        rows.iter().all(|row| !row.starts_with("write: ")),
        "no write row is stated: {rows:?}"
    );
}

/// The card states where the job runs: in the session's own folder beside its
/// run, or in a checkout of its own.
#[test]
fn the_consent_states_where_the_job_runs() {
    let rows: Vec<String> = text("pty-job", &proposal())
        .split('\n')
        .map(str::to_owned)
        .collect();
    assert!(
        rows.contains(&"runs: in this folder, alongside the session".to_owned()),
        "an in-place proposal says the job shares the session's folder: {rows:?}"
    );

    let mut checked_out = proposal();
    checked_out["mode"] = json!("managed");
    let rows: Vec<String> = text("pty-job", &checked_out)
        .split('\n')
        .map(str::to_owned)
        .collect();
    assert!(
        rows.contains(&"runs: in a separate checkout".to_owned()),
        "a checkout proposal says the job works apart from the folder: {rows:?}"
    );
}

#[test]
fn a_proposal_too_long_to_read_is_cut_on_a_character_boundary() {
    let mut arguments = proposal();
    arguments["task"] = json!("✓".repeat(16 * 1024));
    let stated = text("pty-job", &arguments);
    assert!(stated.len() <= 16 * 1024, "the consent stays bounded");
    assert!(
        stated.starts_with("Approve job pty-job"),
        "and keeps its head"
    );
}
