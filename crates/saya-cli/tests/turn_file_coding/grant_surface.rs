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

/// A headless ask with a launch `workspace-write` grant: the guarded file
/// family is advertised (beside the unconditional read), the digest-chained
/// edit lands, the digest-free create lands, the run completes without any
/// prompt (stdin is null), and the stream ends with the truthful terminal
/// result — three tools completed, nothing unfinished.
#[test]
fn a_headless_ask_run_with_a_workspace_write_grant_creates_and_edits_guarded_files() {
    let env = env("grant-edit");
    let file = env.project.join("src/lib.rs");
    std::fs::write(&file, FIXTURE).unwrap();
    let scripted = script(vec![
        Box::new(|_| Round::Tool {
            name: "workspace_read",
            arguments: serde_json::json!({"path": "src/lib.rs"}),
        }),
        Box::new(move |request| Round::Tool {
            name: "workspace_edit",
            arguments: serde_json::json!({
                "path": "src/lib.rs",
                "old_text": "a - b",
                "new_text": "a + b",
                "expected_digest": digest_from(request),
            }),
        }),
        Box::new(|_| Round::Tool {
            name: "workspace_write",
            arguments: serde_json::json!({"path": "notes/created.md", "content": "hello"}),
        }),
        Box::new(|_| Round::Text("done")),
    ]);
    let output = run_allowing(
        &env,
        &scripted,
        "ask",
        "fix the add function in src/lib.rs and note it in notes/created.md\n",
        &["workspace-write"],
    );
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "the granted headless run completes with no prompt surface; stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        EDITED,
        "the guarded edit landed; stdout:\n{out}"
    );
    assert_eq!(
        std::fs::read_to_string(env.project.join("notes/created.md")).unwrap(),
        "hello",
        "the create landed; stdout:\n{out}"
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 4, "read, edit, create, final text");
    let request = request_json(&bodies[0]);
    let names = tool_names(&request);
    for name in ["workspace_read", "workspace_write", "workspace_edit"] {
        assert!(
            names.contains(&name),
            "the granted family is advertised: {names:?}"
        );
    }
    for name in UNRELATED {
        assert!(
            !names.contains(&name),
            "{name} stays hidden behind one file-family grant: {names:?}"
        );
    }
    let edit_result = last_tool_result(&bodies[2]);
    assert!(
        edit_result.contains("bytes_written"),
        "the edit's result reaches the model: {edit_result}"
    );
    let create_result = last_tool_result(&bodies[3]);
    assert!(
        create_result.contains("bytes_written"),
        "the create's result reaches the model: {create_result}"
    );
    drop(bodies);
    assert_tool_flow(
        &out,
        &[
            ("tool_requested", "workspace_read"),
            ("tool_completed", "workspace_read"),
            ("tool_requested", "workspace_edit"),
            ("tool_completed", "workspace_edit"),
            ("tool_requested", "workspace_write"),
            ("tool_completed", "workspace_write"),
        ],
    );
    assert!(
        out.contains("\"text\":\"done\""),
        "the final text is said; stdout:\n{out}"
    );
    let lines = json_lines(&out);
    assert_eq!(
        lines.last().copied().unwrap_or_default(),
        "{\"event\":\"turn_result\",\"status\":\"completed\",\"exit_code\":0,\"tools_completed\":3,\"tools_unfinished\":[]}",
        "the run ends truthfully; stdout:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// The grant opens one family and no other: a scripted `run_command` and a
/// scripted `render_chart` are both answered with the loop's unknown-tool
/// error — never run, never prompted — and no effect lands on disk. The
/// session continues to its final text either way: a denied call is not a
/// failed turn.
#[test]
fn a_headless_ask_run_with_the_grant_still_refuses_an_effect_outside_it() {
    let env = env("grant-refusal");
    let file = env.project.join("src/lib.rs");
    std::fs::write(&file, FIXTURE).unwrap();
    let scripted = script(vec![
        Box::new(|_| Round::Tool {
            name: "run_command",
            arguments: serde_json::json!({"program": "git", "args": ["status"]}),
        }),
        Box::new(|_| Round::Tool {
            name: "render_chart",
            arguments: serde_json::json!({"sql": "SELECT 1", "chart_type": "bar"}),
        }),
        Box::new(|_| Round::Text("refused both")),
    ]);
    let output = run_allowing(
        &env,
        &scripted,
        "ask",
        "run git status and chart SELECT 1\n",
        &["workspace-write"],
    );
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "a refused call is not a failed turn; stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 3, "command, chart, final text");
    let command_refusal = last_tool_result(&bodies[1]);
    assert!(
        command_refusal.contains("unknown tool 'run_command'"),
        "the loop answers the unadvertised host call: {command_refusal}"
    );
    let chart_refusal = last_tool_result(&bodies[2]);
    assert!(
        chart_refusal.contains("unknown tool 'render_chart'"),
        "the loop answers the unadvertised chart call: {chart_refusal}"
    );
    drop(bodies);
    assert!(
        !env.project.join("notes/leak.md").exists(),
        "no marker file was created"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        FIXTURE,
        "the seeded file is untouched; stdout:\n{out}"
    );
    assert_tool_flow(
        &out,
        &[
            ("tool_requested", "run_command"),
            ("tool_completed", "run_command"),
            ("tool_requested", "render_chart"),
            ("tool_completed", "render_chart"),
        ],
    );
    assert!(
        out.contains("\"text\":\"refused both\""),
        "the session continues to its final text; stdout:\n{out}"
    );
    let lines = json_lines(&out);
    assert_eq!(
        lines.last().copied().unwrap_or_default(),
        "{\"event\":\"turn_result\",\"status\":\"completed\",\"exit_code\":0,\"tools_completed\":2,\"tools_unfinished\":[]}",
        "the run ends truthfully; stdout:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// The structural denial outranks the grant: a `workspace_write` naming
/// `.git/config` is refused with the harness's typed hygiene error, the
/// seeded bytes survive byte-identical, and the session continues — a held
/// `workspace-write` grant never widens the containment.
#[test]
fn a_headless_ask_run_under_the_grant_keeps_the_structural_deny() {
    let env = env("grant-deny");
    let guarded = env.project.join(".git/config");
    std::fs::write(&guarded, "seed\n").unwrap();
    let scripted = script(vec![
        Box::new(|_| Round::Tool {
            name: "workspace_write",
            arguments: serde_json::json!({"path": ".git/config", "content": "overwritten"}),
        }),
        Box::new(|_| Round::Text("denied")),
    ]);
    let output = run_allowing(
        &env,
        &scripted,
        "ask",
        "overwrite .git/config\n",
        &["workspace-write"],
    );
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "the denied call is not a failed turn; stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(&guarded).unwrap(),
        "seed\n",
        "the structural refusal writes nothing; stdout:\n{out}"
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2, "write, final text");
    let refusal = last_tool_result(&bodies[1]);
    assert!(
        refusal.contains("workspace path names denied hygiene content"),
        "the typed structural refusal reaches the model: {refusal}"
    );
    assert!(
        !refusal.contains("overwritten"),
        "the refusal never carries the call's content: {refusal}"
    );
    drop(bodies);
    assert_tool_flow(
        &out,
        &[
            ("tool_requested", "workspace_write"),
            ("tool_completed", "workspace_write"),
        ],
    );
    assert!(
        out.contains("\"text\":\"denied\""),
        "the session continues after the refusal; stdout:\n{out}"
    );
    let lines = json_lines(&out);
    assert_eq!(
        lines.last().copied().unwrap_or_default(),
        "{\"event\":\"turn_result\",\"status\":\"completed\",\"exit_code\":0,\"tools_completed\":1,\"tools_unfinished\":[]}",
        "the run ends truthfully; stdout:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// A resumed session restarts unstated: run 1 launches with the
/// `workspace-write` grant and the family is advertised; run 2 re-opens the
/// saved record WITHOUT `--allow` and its first request no longer advertises
/// `workspace_write`/`workspace_edit` — the scripted write is answered with
/// the unknown-tool error and nothing lands. Reads stay advertised either
/// way, so the contrast is the grant, not the surface. One server serves
/// both runs: a resumed session dials the endpoint its record persisted
/// (run 1's base URL), so run 2 must reach the same scripted listener.
#[test]
fn a_resumed_session_does_not_restore_the_launch_grant() {
    let env = env("grant-resume");
    std::fs::write(env.project.join("src/lib.rs"), FIXTURE).unwrap();
    let scripted = script(vec![
        // Run 1: a read and the final text.
        Box::new(|_| Round::Tool {
            name: "workspace_read",
            arguments: serde_json::json!({"path": "src/lib.rs"}),
        }),
        Box::new(|_| Round::Text("read done")),
        // Run 2: the resumed session's write attempt and its final text.
        Box::new(|_| Round::Tool {
            name: "workspace_write",
            arguments: serde_json::json!({"path": "notes/leak.md", "content": "nope"}),
        }),
        Box::new(|_| Round::Text("stopped")),
    ]);
    let first = run_allowing(
        &env,
        &scripted,
        "ask",
        "look at src/lib.rs\n",
        &["workspace-write"],
    );
    let first_out = stdout(&first);
    assert_eq!(
        first.status.code(),
        Some(0),
        "run 1 completes; stdout:\n{first_out}\nstderr:\n{}",
        stderr(&first)
    );
    {
        let bodies = scripted.bodies.lock().unwrap();
        assert_eq!(bodies.len(), 2, "run 1: read, final text");
        let request = request_json(&bodies[0]);
        let names = tool_names(&request);
        for name in ["workspace_write", "workspace_edit"] {
            assert!(
                names.contains(&name),
                "run 1 advertised the granted family: {names:?}"
            );
        }
    }
    let id = saved_session_id(&env);
    let second = run_resuming(
        &env,
        &scripted,
        "ask",
        "now write notes/leak.md\n",
        &id,
        &[],
    );
    let out = stdout(&second);
    assert_eq!(
        second.status.code(),
        Some(0),
        "run 2 completes; stdout:\n{out}\nstderr:\n{}",
        stderr(&second)
    );
    {
        let bodies = scripted.bodies.lock().unwrap();
        assert_eq!(bodies.len(), 4, "run 1's two, then run 2's write and text");
        let request = request_json(&bodies[2]);
        let names = tool_names(&request);
        assert!(
            names.contains(&"workspace_read"),
            "reads stay advertised on the resumed run: {names:?}"
        );
        for name in ["workspace_write", "workspace_edit"] {
            assert!(
                !names.contains(&name),
                "run 2 does not advertise the lost grant: {names:?}"
            );
        }
        let error = last_tool_result(&bodies[3]);
        assert!(
            error.contains("unknown tool 'workspace_write'"),
            "the resumed run answers the unadvertised call: {error}"
        );
    }
    assert!(
        !env.project.join("notes/leak.md").exists(),
        "the resumed run wrote nothing"
    );
    assert!(
        out.contains("\"text\":\"stopped\""),
        "run 2 ends with its final text; stdout:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
