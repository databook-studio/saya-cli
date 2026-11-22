//! The fixtures the resume-path rows share: a session the real binary saved,
//! the job its record links, and the resumed process's first request.
//!
//! Every row starts the same way — one answered turn through the binary, then
//! `/exit` — so the record on disk is one the store itself wrote, and the row
//! then changes exactly the fact its decision reads: the job's terminal
//! outcome, the link, or the folder the link names.

use crate::harness::{Env, Scripted, request_json};
use crate::piped::{Piped, complete_events};
use crate::support::{last_user_content, project_files, saved_record, single_session_id};
use serde_json::Value;
use std::path::PathBuf;

/// The first process's one turn.
pub(crate) const FIRST_LINE: &str = "finish the first turn";
/// The answer it settled on: the pair a resumed run is seeded with.
pub(crate) const FIRST_ANSWER: &str = "the first answer";
/// The line the resumed process is given.
pub(crate) const RESUME_LINE: &str = "carry on";

/// The environment every row opens: the synthetic project and the config home
/// the binary reads.
pub(crate) fn fixture(name: &str) -> Env {
    let env = crate::harness::env(name);
    std::fs::create_dir_all(env.root.join("cfg")).unwrap();
    project_files(&env);
    env
}

/// One session through the real binary: one answered line, then `/exit`.
/// Returns the saved session's id and the job its record links.
pub(crate) fn first_process(env: &Env, scripted: &Scripted) -> (String, String) {
    let mut piped = Piped::spawn(env, scripted, &[]);
    piped.line(FIRST_LINE);
    complete_events(&piped, 1);
    piped.line("/exit");
    let (status, stdout, stderr) = piped.finish();
    assert!(
        status.success(),
        "the first process exited cleanly\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let id = single_session_id(env);
    let record = saved_record(env, &id);
    assert!(
        !record["turns"]
            .as_array()
            .expect("the record keeps its settled turns")
            .is_empty(),
        "the first process kept the turn the resume is seeded with: {record}"
    );
    let job = linked_job(&record);
    (id, job)
}

/// `saya --resume <id>`: one line, then `/exit`.
pub(crate) struct Resumed {
    pub(crate) status: std::process::ExitStatus,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) request: Value,
}

pub(crate) fn resume(env: &Env, scripted: &Scripted, id: &str, line: &str) -> Resumed {
    let first_new_request = scripted.bodies.lock().unwrap().len();
    let mut piped = Piped::spawn(env, scripted, &["--resume", id]);
    piped.line(line);
    complete_events(&piped, 1);
    piped.line("/exit");
    let (status, stdout, stderr) = piped.finish();
    let request = scripted
        .bodies
        .lock()
        .unwrap()
        .get(first_new_request)
        .map(|raw| request_json(raw))
        .unwrap_or(Value::Null);
    Resumed {
        status,
        stdout,
        stderr,
        request,
    }
}

/// `saya --resume <id>` that answers nothing: the resumed process is expected
/// to say its own sentence and leave.
pub(crate) fn resume_without_a_turn(env: &Env, scripted: &Scripted, id: &str) -> Resumed {
    let mut piped = Piped::spawn(env, scripted, &["--resume", id]);
    piped.line("carry on");
    let (status, stdout, stderr) = piped.finish();
    Resumed {
        status,
        stdout,
        stderr,
        request: Value::Null,
    }
}

/// The job the record links.
pub(crate) fn linked_job(record: &Value) -> String {
    record["run"]["job"]
        .as_str()
        .expect("the saved record links its run's job")
        .to_owned()
}

/// The job's private records, under the environment's state root.
pub(crate) fn job_dir(env: &Env, job: &str) -> PathBuf {
    env.root.join("coding").join(job)
}

/// Rewrites the saved record with `edit` applied, and returns it.
pub(crate) fn edit_record(env: &Env, id: &str, edit: impl FnOnce(&mut Value)) -> Value {
    let mut record = saved_record(env, id);
    edit(&mut record);
    let path = env.sessions.join(format!("{id}.json"));
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    record
}

/// A job the binary itself started and finished in place, under `id`: a run
/// that is over, which a record can link.
pub(crate) fn finished_job(env: &Env, scripted: &Scripted, id: &str) -> String {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "start",
            id,
            "finish this job",
            "--write",
            ".",
            "--in-place",
            "--approve",
            "--allow-data-sharing",
            "--format",
            "ndjson",
        ])
        .current_dir(&env.project)
        .env("SAYA_CONFIG_HOME", env.root.join("cfg"))
        .env("SAYA_SESSION_DIR", &env.sessions)
        .env("SAYA_STATE_DB", env.root.join("state.sqlite3"))
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{}/v1", scripted.address))
        .env("SAYA_API_KEY", "mock-secret")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "the job started and finished\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    id.to_owned()
}

/// What the resumed process's request carried from the earlier conversation.
pub(crate) fn carried_answer(request: &Value) -> bool {
    request.to_string().contains(FIRST_ANSWER) && !last_user_content(request).contains(FIRST_ANSWER)
}
