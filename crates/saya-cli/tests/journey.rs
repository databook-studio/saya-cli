//! Q01 acceptance: one synthetic project journey across TWO processes —
//! instructions + preference → edit with the digest-guarded tool → compact →
//! notes → exit; then `--resume` in a new process after the file changed on
//! disk. Deterministic mock provider; observable outcomes only: the edited
//! file, the saved session record, the request bodies, the REPL's events,
//! and the exit codes.

#[path = "turn_file_coding/harness.rs"]
#[allow(dead_code)] // shared test harness: this file uses only part of it
mod harness;

#[path = "journey/piped.rs"]
mod piped;

#[path = "journey/support.rs"]
mod support;

#[path = "journey/resume_tests.rs"]
mod resume_tests;

use support::*;

#[test]
fn journey_instructions_edit_compact_notes_then_resume_after_the_file_changed() {
    let env = env("q01-journey");
    std::fs::create_dir_all(env.root.join("cfg")).unwrap();
    let file = project_files(&env);

    // ---- process 1: instructions, preference, edit turn, compaction, notes ----
    let scripted = journey_script();
    let (piped, session_id) = run_first_process(&env, &scripted, Some(NOTES_TEXT));
    exit_ok(piped);

    // The saved record: the notes and the file fact ride the typed
    // continuation, never the plain turn text.
    let saved = saved_record(&env, &session_id);
    let record = &saved["continuation"];
    assert_eq!(record["user_notes"].as_str(), Some(NOTES_TEXT));
    assert!(
        record["narrative"]["text"]
            .as_str()
            .unwrap_or_default()
            .contains(SUMMARY_TEXT),
        "the compaction summary is the saved narrative"
    );
    let fact = record["files"]
        .as_array()
        .expect("file facts are saved")
        .iter()
        .find(|fact| fact["path"] == "src/lib.rs")
        .expect("the edited file is a saved fact")
        .clone();
    assert_eq!(
        fact["sha256"].as_str().unwrap().len(),
        64,
        "digest recorded"
    );
    for turn in saved["turns"].as_array().expect("turns are saved") {
        let text = format!(
            "{} {}",
            turn["user"].as_str().unwrap_or_default(),
            turn["assistant"].as_str().unwrap_or_default()
        );
        assert!(
            !text.contains(NOTES_TEXT),
            "notes never ride plain turn text"
        );
    }

    // Between processes: someone else rewrites the file on disk.
    std::fs::write(&file, "pub fn add(a: i32, b: i32) -> i32 { a * b }\n").unwrap();

    // ---- process 2: --resume, /context, then the prose turn ----
    let resumed = resume_and_run(&env, &scripted, &session_id, &["/context"], &["/recovery"]);
    let user = last_user_content(&resumed.first_request);
    let system = system_content(&resumed.first_request);

    // The restored summary rides the compaction-narrative block.
    assert!(user.contains("source: compaction-narrative"), "{user}");
    assert!(user.contains(SUMMARY_TEXT), "{user}");
    // The saved record block: notes, the file fact now changed, and NO
    // historical checks section (this journey ran no command).
    assert!(user.contains("source: continuation-record"), "{user}");
    assert!(user.contains(NOTES_TEXT), "{user}");
    assert!(
        user.contains("src/lib.rs — changed since observed"),
        "{user}"
    );
    assert!(!user.contains("Checks run earlier"), "{user}");
    // Instructions and preferences: the stable prefix, each exactly once,
    // ahead of the volatile blocks.
    assert_eq!(
        user.matches("source: project-instructions (AGENTS.md)")
            .count(),
        1,
        "{user}"
    );
    assert_eq!(
        user.matches("source: project-preferences").count(),
        1,
        "{user}"
    );
    assert!(user.contains(PREFERENCE_TEXT), "{user}");
    let instructions_at = user
        .find("source: project-instructions (AGENTS.md)")
        .unwrap();
    let preferences_at = user.find("source: project-preferences").unwrap();
    let summary_at = user.find("source: compaction-narrative").unwrap();
    let record_at = user.find("source: continuation-record").unwrap();
    assert!(instructions_at < summary_at && preferences_at < summary_at);
    assert!(summary_at < record_at);
    // The untrusted lane keeps the notes, the preference, and the
    // instruction body out of the system prompt.
    assert!(!system.contains(NOTES_TEXT), "{system}");
    assert!(!system.contains(PREFERENCE_TEXT), "{system}");
    assert!(!system.contains(INSTRUCTIONS_BODY_LINE), "{system}");
    // The compacted region is gone from history; the kept tail stays.
    let request_text = resumed.first_request.to_string();
    assert!(
        !request_text.contains("fix the add function"),
        "{request_text}"
    );
    assert!(request_text.contains("filler turn"), "{request_text}");
    // The tool surface is what bypass advertises.
    assert!(
        tool_names(&resumed.first_request).contains(&"workspace_edit"),
        "{request_text}"
    );
    // /context lists the instruction source without its body.
    assert!(resumed.stdout.contains("AGENTS.md"), "{}", resumed.stdout);
    assert!(
        !resumed.stdout.contains(INSTRUCTIONS_BODY_LINE),
        "{}",
        resumed.stdout
    );
    // The recovery flag does not survive the process boundary.
    assert!(
        resumed.stdout.contains("Recovery is off"),
        "{}",
        resumed.stdout
    );

    cleanup(&env);
}
