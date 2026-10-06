//! Shared fixtures and drivers for the Q01 journey tests: the synthetic
//! project, the first process (preference → digest-guarded edit → filler
//! turns → /compact → /notes), the resumed second process, and the readers
//! for the saved session record and the captured request bodies.

use crate::harness::{Env, Round, Scripted, digest_from, request_json};
use crate::piped::{Piped, complete_events};
use serde_json::Value;
use std::path::PathBuf;

pub(crate) use crate::harness::{env, script, tool_names};

pub(crate) const NOTES_TEXT: &str = "keep the public API unchanged";
pub(crate) const PREFERENCE_TEXT: &str = "prefer small diffs";
pub(crate) const INSTRUCTIONS_BODY_LINE: &str = "Run the formatter before finishing.";
pub(crate) const SUMMARY_TEXT: &str = "Summary: fixed add() in src/lib.rs; formatter still to run.";
const PINNED_HEADING: &str = "Pinned tokens (preserve every one verbatim):";

/// The synthetic project: AGENTS.md instructions and the file the journey
/// edits. Returns the edited file's path.
pub(crate) fn project_files(env: &Env) -> PathBuf {
    std::fs::write(
        env.project.join("AGENTS.md"),
        "# Project rules\nPrefer small diffs. Run the formatter before finishing.\n",
    )
    .unwrap();
    let file = env.project.join("src/lib.rs");
    std::fs::write(&file, "pub fn add(a: i32, b: i32) -> i32 { a - b }\n").unwrap();
    file
}

/// The journey's provider script, serving BOTH processes: process 1's
/// rounds (read → digest-guarded edit → final text; three one-round filler
/// turns; the compaction summariser) plus process 2's "continue" round. One
/// server, because the session binds its launch endpoint at startup and a
/// resumed session keeps that persisted endpoint (resume continuity) — the
/// second process must reopen the same address, and the rounds are consumed
/// in order. The summary preserves every pinned token the summariser prompt
/// asks for verbatim.
pub(crate) fn journey_script() -> Scripted {
    script(vec![
        Box::new(|_| Round::Tool {
            name: "workspace_read",
            arguments: serde_json::json!({"path": "src/lib.rs"}),
        }),
        Box::new(|previous| Round::Tool {
            name: "workspace_edit",
            arguments: serde_json::json!({
                "path": "src/lib.rs",
                "old_text": "a - b",
                "new_text": "a + b",
                "expected_digest": digest_from(previous)
            }),
        }),
        Box::new(|_| Round::Text("fixed add")),
        Box::new(|_| Round::Text("turn two done")),
        Box::new(|_| Round::Text("turn three done")),
        Box::new(|_| Round::Text("turn four done")),
        Box::new(move |previous: &str| {
            let mut summary = String::from(SUMMARY_TEXT);
            for token in pinned_tokens(previous) {
                summary.push('\n');
                summary.push_str(&token);
            }
            Round::Text(Box::leak(summary.into_boxed_str()))
        }),
        Box::new(|_| Round::Text("ok")),
    ])
}

/// The pinned-token lines of the summariser prompt, read out of the raw HTTP
/// request: the prompt rides a JSON string, so lines are its escaped `\n`
/// sequences. The journey's conversation carries no anchors, so the list is
/// usually empty — the parse is defensive, not load-bearing.
fn pinned_tokens(summarise_request: &str) -> Vec<String> {
    let Some(at) = summarise_request.find(PINNED_HEADING) else {
        return Vec::new();
    };
    let mut tokens = Vec::new();
    for segment in summarise_request[at + PINNED_HEADING.len()..].split("\\n") {
        if segment.is_empty() {
            if tokens.is_empty() {
                continue;
            }
            break;
        }
        match segment.trim_start().strip_prefix("- ") {
            Some(token) => tokens.push(token.trim_end_matches(['"', '}', ']']).to_owned()),
            None => break,
        }
    }
    tokens
}

/// Drives the journey's first process: the project preference, the read →
/// edit turn (the file ends up edited), three filler turns, `/compact`
/// (which asks the mock for a summary), optional `/notes set`, and leaves
/// the child alive (the caller owns the rest). Returns the child and the
/// saved session id — the single `*.json` stem in the sessions dir at that
/// point.
pub(crate) fn run_first_process(
    env: &Env,
    scripted: &Scripted,
    notes: Option<&str>,
) -> (Piped, String) {
    let mut piped = Piped::spawn(env, scripted, &[]);
    piped.line(&format!("/preferences add {PREFERENCE_TEXT}"));
    piped.wait_for("Added preference");
    piped.line("fix the add function");
    complete_events(&piped, 1);
    assert_eq!(
        std::fs::read_to_string(env.project.join("src/lib.rs")).unwrap(),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
        "the edit round wrote through the digest-guarded tool"
    );
    for nth in 2..=4 {
        piped.line("filler turn");
        complete_events(&piped, nth);
    }
    piped.line("/compact");
    piped.wait_for("Compacted 2 turns");
    if let Some(notes) = notes {
        piped.line(&format!("/notes set {notes}"));
        piped.wait_for("Session notes stored.");
    }
    (piped, single_session_id(env))
}

/// Exits the first process cleanly, returning its captured output.
pub(crate) fn exit_ok(piped: Piped) -> String {
    let mut piped = piped;
    piped.line("/exit");
    let (status, stdout, stderr) = piped.finish();
    assert!(
        status.success(),
        "the first process exited cleanly\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    stdout
}

/// The resumed second process: `--resume <id>`, the caller's lines, the
/// prose turn "continue", then the after-lines. `scripted` is the SAME
/// server process 1 used (the session's persisted endpoint pins the
/// address); its first new request is the turn's FIRST request body.
pub(crate) fn resume_and_run(
    env: &Env,
    scripted: &Scripted,
    session_id: &str,
    before: &[&str],
    after: &[&str],
) -> Resumed {
    let first_new_request = scripted.bodies.lock().unwrap().len();
    let mut piped = Piped::spawn(env, scripted, &["--resume", session_id]);
    for line in before {
        piped.line(line);
    }
    piped.line("continue");
    complete_events(&piped, 1);
    for line in after {
        piped.line(line);
    }
    let raw = scripted.bodies.lock().unwrap()[first_new_request].clone();
    let first_request = request_json(&raw);
    let (status, stdout, stderr) = piped.finish();
    assert!(
        status.success(),
        "the resumed process exited cleanly\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    Resumed {
        stdout,
        first_request,
    }
}

pub(crate) struct Resumed {
    pub(crate) stdout: String,
    pub(crate) first_request: Value,
}

/// The saved session id: the single `*.json` file's stem in the sessions dir.
pub(crate) fn single_session_id(env: &Env) -> String {
    let mut records = session_record_paths(env);
    assert_eq!(
        records.len(),
        1,
        "exactly one saved session record: {records:?}"
    );
    records
        .remove(0)
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn session_record_paths(env: &Env) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&env.sessions)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    paths
}

/// The parsed saved session record for `id`.
pub(crate) fn saved_record(env: &Env, id: &str) -> Value {
    let path = env.sessions.join(format!("{id}.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
        .expect("the saved session record parses")
}

/// The request's system prompt (the untrusted-lane texts must stay out of it).
pub(crate) fn system_content(request: &Value) -> &str {
    request["messages"]
        .as_array()
        .expect("request carries messages")
        .iter()
        .find(|message| message["role"] == "system")
        .and_then(|message| message["content"].as_str())
        .expect("the request carries a system prompt")
}

/// The current user turn: the last user message, which carries the context
/// blocks and the prompt.
pub(crate) fn last_user_content(request: &Value) -> &str {
    request["messages"]
        .as_array()
        .expect("request carries messages")
        .iter()
        .rev()
        .find(|message| message["role"] == "user")
        .and_then(|message| message["content"].as_str())
        .expect("the request carries a user turn")
}

/// Removes the test's temp root; call at the end of every test.
pub(crate) fn cleanup(env: &Env) {
    let _ = std::fs::remove_dir_all(&env.root);
}
