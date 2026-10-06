//! The Q01 focused resume scenarios: each drives the same synthetic journey
//! through the real binary and asserts one observable outcome of `--resume`.

use crate::piped::Piped;
use crate::support::*;

/// Same journey, no external rewrite: the record block marks the file
/// unchanged since it was observed.
#[test]
fn resume_without_a_file_change_marks_the_file_unchanged() {
    let env = env("q01-resume-unchanged");
    std::fs::create_dir_all(env.root.join("cfg")).unwrap();
    project_files(&env);
    let scripted = journey_script();
    let (piped, session_id) = run_first_process(&env, &scripted, Some(NOTES_TEXT));
    exit_ok(piped);

    let resumed = resume_and_run(&env, &scripted, &session_id, &[], &[]);
    let user = last_user_content(&resumed.first_request);
    assert!(user.contains("source: continuation-record"), "{user}");
    assert!(
        user.contains("src/lib.rs — unchanged since observed"),
        "{user}"
    );
    // "unchanged since observed" carries "changed since observed" as a
    // substring; the em-dash line prefix is what only the changed status
    // renders.
    assert!(
        !user.contains("src/lib.rs — changed since observed"),
        "{user}"
    );

    cleanup(&env);
}

/// The file deleted between the processes: the record block reports the
/// fact as not found, not as changed.
#[test]
fn a_deleted_file_is_not_found_after_resume() {
    let env = env("q01-resume-deleted");
    std::fs::create_dir_all(env.root.join("cfg")).unwrap();
    project_files(&env);
    let scripted = journey_script();
    let (piped, session_id) = run_first_process(&env, &scripted, Some(NOTES_TEXT));
    exit_ok(piped);

    std::fs::remove_file(env.project.join("src/lib.rs")).unwrap();
    let resumed = resume_and_run(&env, &scripted, &session_id, &[], &[]);
    let user = last_user_content(&resumed.first_request);
    assert!(user.contains("source: continuation-record"), "{user}");
    assert!(user.contains("src/lib.rs — not found"), "{user}");
    assert!(
        !user.contains("src/lib.rs — changed since observed"),
        "{user}"
    );
    assert!(
        !user.contains("src/lib.rs — unchanged since observed"),
        "{user}"
    );

    cleanup(&env);
}

/// Session grants and the recovery flag are process-lifetime facts: neither
/// survives `--resume`, and the resumed session says so in its own words.
#[test]
fn grants_and_recovery_do_not_survive_resume() {
    let env = env("q01-resume-grants");
    std::fs::create_dir_all(env.root.join("cfg")).unwrap();
    project_files(&env);
    // No provider calls: the grant and the flag are slash-command turns.
    let scripted = script(vec![]);
    let mut piped = Piped::spawn(&env, &scripted, &[]);
    piped.line("/allow workspace-write");
    piped.wait_for("granted for this session (dies with it): workspace-write");
    piped.line("/recovery on");
    piped.wait_for("Recovery is on.");
    piped.line("/exit");
    let (status, stdout, stderr) = piped.finish();
    assert!(status.success(), "{stdout}\n{stderr}");
    let session_id = single_session_id(&env);

    let resumed_scripted = script(vec![]);
    let mut resumed = Piped::spawn(&env, &resumed_scripted, &["--resume", &session_id]);
    resumed.line("/grants");
    resumed.wait_for("(none — nothing pre-answers this session yet");
    resumed.line("/recovery");
    resumed.wait_for("Recovery is off");
    resumed.line("/exit");
    let (status, stdout, stderr) = resumed.finish();
    assert!(status.success(), "{stdout}\n{stderr}");

    cleanup(&env);
}

/// `/rotate` into a successor: the summary and the notes ride, the parent
/// record stays on disk, and the successor holds no grants.
#[test]
fn rotation_carries_summary_and_notes_but_no_authority() {
    let env = env("q01-resume-rotate");
    std::fs::create_dir_all(env.root.join("cfg")).unwrap();
    project_files(&env);
    let scripted = journey_script();
    let (piped, parent_id) = run_first_process(&env, &scripted, Some(NOTES_TEXT));
    let mut piped = piped;
    piped.line("/rotate");
    piped.wait_for("Rotated into a new session");
    let successor = successor_id(&piped.stdout());
    piped.line("/exit");
    let (status, stdout, stderr) = piped.finish();
    assert!(status.success(), "{stdout}\n{stderr}");
    assert!(
        env.sessions.join(format!("{parent_id}.json")).exists(),
        "the parent session file survives the rotation"
    );
    assert!(env.sessions.join(format!("{successor}.json")).exists());

    let resumed = resume_and_run(&env, &scripted, &successor, &[], &["/grants"]);
    let user = last_user_content(&resumed.first_request);
    assert!(user.contains("source: compaction-narrative"), "{user}");
    assert!(user.contains(SUMMARY_TEXT), "{user}");
    assert!(user.contains("source: continuation-record"), "{user}");
    assert!(user.contains(NOTES_TEXT), "{user}");
    assert!(
        resumed
            .stdout
            .contains("(none — nothing pre-answers this session yet"),
        "{}",
        resumed.stdout
    );

    cleanup(&env);
}

/// The successor id from the rotation message ("Rotated into a new session
/// <id> (a successor of <old>).").
fn successor_id(stdout: &str) -> String {
    let after = "Rotated into a new session ";
    let at = stdout
        .find(after)
        .expect("the rotation names its successor");
    stdout[at + after.len()..]
        .chars()
        .take_while(|character| !character.is_whitespace() && *character != '(')
        .collect()
}
