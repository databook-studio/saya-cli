//! The scoped-grant proofs through the real turn-file lane: a fresh launch's
//! `--allow workspace-write` seeds exactly the guarded file family — under a
//! headless `ask` the granted tools are advertised, run without prompting,
//! and their effects land on disk, while every other family stays hidden and
//! an out-of-grant call is refused without prompting, the structural denial
//! still wins under the grant, and a resumed session restarts unstated (the
//! grant dies with the process). Every test asserts on the captured request
//! bodies and the real filesystem — never on the tool tally alone, which a
//! model can complete after a denied call.

use super::harness::{
    Env, Round, digest_from, env, last_tool_result, request_json, run_allowing, run_resuming,
    script, stderr, stdout, tool_names,
};
use super::{EDITED, FIXTURE, assert_tool_flow, json_lines};

/// The families one `workspace-write` grant must NOT open under a headless
/// ask: the chart save, the host/runner lanes, scratch, and fetch. The grant
/// pre-answers the file family's calls alone.
const UNRELATED: [&str; 6] = [
    "render_chart",
    "run_command",
    "run_program",
    "scratch_sql",
    "http_fetch",
    "http_download",
];

/// The one session record run 1 saved: its filename stem is the id a
/// `--resume` re-opens.
fn saved_session_id(env: &Env) -> String {
    let mut records: Vec<std::path::PathBuf> = std::fs::read_dir(&env.sessions)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    records.sort();
    assert_eq!(
        records.len(),
        1,
        "run 1 saved exactly one session record: {records:?}"
    );
    records[0]
        .file_stem()
        .expect("the record names its id")
        .to_string_lossy()
        .into_owned()
}

include!("grant_surface/granted_files.rs");

include!("grant_surface/out_of_grant_refusal.rs");

include!("grant_surface/structural_denial.rs");

include!("grant_surface/resume_scope.rs");
