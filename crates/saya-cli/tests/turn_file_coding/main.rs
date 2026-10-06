//! Scripted no-database coding through `--workspace --turn-file`: a mock
//! OpenAI-compatible provider answers each round with either a tool call or a
//! final text, and a round may depend on the request body it answers (the
//! digest the model "observed" comes from the read tool's result in that very
//! body). These tests pin, through the real `saya` binary, that a headless
//! one-turn session reads a file, edits it with the digest-guarded tool, and
//! reports — with NO database profile and no `--connections`.

mod harness;

use harness::{
    Round, RoundFn, digest_from, env, request_json, run, script, stdout, tool_names, tool_results,
};

const FIXTURE: &str = "pub fn add(a: i32, b: i32) -> i32 { a - b }\n";
const EDITED: &str = "pub fn add(a: i32, b: i32) -> i32 { a + b }\n";

/// Every stdout line must parse as JSON (ndjson stays machine-readable), the
/// tool events must arrive requested-then-completed in order, and the stream
/// must end complete.
fn assert_tool_flow(out: &str, expected: &[(&str, &str)]) {
    for line in out.lines().filter(|line| !line.is_empty()) {
        serde_json::from_str::<serde_json::Value>(line).expect("every stdout line is JSON");
    }
    let pairs: Vec<(String, String)> = out
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|event| {
            let kind = event["event"].as_str()?;
            let name = event["name"].as_str()?;
            matches!(kind, "tool_requested" | "tool_completed" | "tool_denied")
                .then(|| (kind.to_owned(), name.to_owned()))
        })
        .collect();
    let expected: Vec<(String, String)> = expected
        .iter()
        .copied()
        .map(|(kind, name)| (kind.to_owned(), name.to_owned()))
        .collect();
    assert_eq!(pairs, expected, "tool events in order; stdout:\n{out}");
    assert!(
        out.lines()
            .last()
            .is_some_and(|line| line.contains("\"event\":\"complete\"")),
        "the stream ends complete; stdout:\n{out}"
    );
}

/// The read → guarded-edit → final-text script. Round 2's `expected_digest`
/// is either the `wrong` value (to probe the guard) or the digest parsed out
/// of the read's tool result in the body the round answers — the model must
/// chain exactly what the tool returned.
fn read_edit_rounds(wrong: Option<String>) -> Vec<RoundFn> {
    vec![
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
                "expected_digest": wrong.clone().unwrap_or_else(|| digest_from(request)),
            }),
        }),
        Box::new(|_| Round::Text("fixed")),
    ]
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The last tool-result message of a captured request: the model-facing
/// result of the call the previous round made.
fn last_tool_result(raw: &str) -> String {
    tool_results(&request_json(raw))
        .last()
        .copied()
        .expect("the request carries a tool result")
        .to_owned()
}

#[test]
fn scripted_read_then_edit_changes_the_file_and_exits_zero() {
    let env = env("edit");
    let file = env.project.join("src/lib.rs");
    std::fs::write(&file, FIXTURE).unwrap();
    let scripted = script(read_edit_rounds(None));
    let output = run(
        &env,
        &scripted,
        "bypass",
        "fix the add function in src/lib.rs\n",
    );
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        EDITED,
        "the edit landed and nothing else changed; stdout:\n{out}"
    );
    assert_tool_flow(
        &out,
        &[
            ("tool_requested", "workspace_read"),
            ("tool_completed", "workspace_read"),
            ("tool_requested", "workspace_edit"),
            ("tool_completed", "workspace_edit"),
        ],
    );
    assert!(
        out.contains("\"text\":\"fixed\""),
        "the final text is said; stdout:\n{out}"
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(
        bodies.len(),
        3,
        "read, edit, final text: exactly three rounds"
    );
    let edit_result = last_tool_result(&bodies[2]);
    assert!(
        edit_result.contains("bytes_written"),
        "the round-3 body carries the edit result: {edit_result}"
    );
    drop(bodies);
    let _ = std::fs::remove_dir_all(&env.root);
}

#[test]
fn a_stale_digest_edit_is_refused_and_leaves_the_file_unchanged() {
    let env = env("stale-digest");
    let file = env.project.join("src/lib.rs");
    std::fs::write(&file, FIXTURE).unwrap();
    let scripted = script(read_edit_rounds(Some("0".repeat(64))));
    let output = run(
        &env,
        &scripted,
        "bypass",
        "fix the add function in src/lib.rs\n",
    );
    let out = stdout(&output);
    // A refused tool call is not a failed turn: the final text still arrives.
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        FIXTURE,
        "the refused edit leaves the file byte-identical; stdout:\n{out}"
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 3, "the loop still reaches the final text");
    // The refusal reaches the model as the edit call's tool result.
    let refusal = last_tool_result(&bodies[2]);
    assert!(
        refusal.contains("file changed since measured"),
        "the refusal names the moved anchor: {refusal}"
    );
    // The refusal must not hand over the file's content.
    assert!(
        !refusal.contains("a - b"),
        "the refusal never carries file content: {refusal}"
    );
    // The edit refusal states the file's CURRENT digest and size (the
    // `WorkspaceEditMoved` contract) so the model can retry after a re-read —
    // it carries state metadata, never content (asserted above). Unlike
    // `workspace_write`'s refusals, which carry no digest at all.
    let observed = digest_from(&bodies[1]);
    assert!(
        refusal.contains(&observed),
        "the edit refusal names the current digest so a retry is possible: {refusal}"
    );
    drop(bodies);
    let _ = std::fs::remove_dir_all(&env.root);
}

#[test]
fn replacing_an_existing_file_without_a_digest_is_refused() {
    let env = env("replace-no-digest");
    let file = env.project.join("src/lib.rs");
    std::fs::write(&file, FIXTURE).unwrap();
    let scripted = script(vec![
        Box::new(|_| Round::Tool {
            name: "workspace_write",
            arguments: serde_json::json!({"path": "src/lib.rs", "content": "x"}),
        }),
        Box::new(|_| Round::Text("left it alone")),
    ]);
    let output = run(&env, &scripted, "bypass", "write src/lib.rs with just x\n");
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        FIXTURE,
        "the refused replace leaves the file byte-identical; stdout:\n{out}"
    );
    assert_tool_flow(
        &out,
        &[
            ("tool_requested", "workspace_write"),
            ("tool_completed", "workspace_write"),
        ],
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2, "write, final text: exactly two rounds");
    let refusal = last_tool_result(&bodies[1]);
    assert!(
        refusal.contains("expected_digest"),
        "the refusal names the guard: {refusal}"
    );
    drop(bodies);
    let _ = std::fs::remove_dir_all(&env.root);
}

#[test]
fn creating_a_new_file_needs_no_digest() {
    let env = env("create-new");
    let scripted = script(vec![
        Box::new(|_| Round::Tool {
            name: "workspace_write",
            arguments: serde_json::json!({"path": "notes/new.md", "content": "hello"}),
        }),
        Box::new(|_| Round::Text("wrote it")),
    ]);
    let output = run(
        &env,
        &scripted,
        "bypass",
        "create notes/new.md with hello\n",
    );
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(env.project.join("notes/new.md")).unwrap(),
        "hello",
        "the file was created with exactly that content; stdout:\n{out}"
    );
    assert_tool_flow(
        &out,
        &[
            ("tool_requested", "workspace_write"),
            ("tool_completed", "workspace_write"),
        ],
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2, "write, final text: exactly two rounds");
    let result = last_tool_result(&bodies[1]);
    assert!(
        result.contains("bytes_written"),
        "the create result reaches the model: {result}"
    );
    drop(bodies);
    let _ = std::fs::remove_dir_all(&env.root);
}

/// A headless run without a prompt surface advertises no write-shaped tool:
/// reads stay advertised, a scripted write call is answered with the loop's
/// "unknown tool" error, and nothing is created — headless coding needs an
/// explicit bypass.
#[test]
fn a_headless_ask_run_advertises_no_write_tool() {
    for mode in ["ask", "read-only"] {
        let env = env("no-write-tool");
        std::fs::write(env.project.join("src/lib.rs"), FIXTURE).unwrap();
        let scripted = script(vec![
            Box::new(|_| Round::Tool {
                name: "workspace_write",
                arguments: serde_json::json!({"path": "notes/leak.md", "content": "nope"}),
            }),
            Box::new(|_| Round::Text("nothing written")),
        ]);
        let output = run(&env, &scripted, mode, "write notes/leak.md with nope\n");
        let out = stdout(&output);
        assert_eq!(
            output.status.code(),
            Some(0),
            "mode {mode}; stdout:\n{out}\nstderr:\n{}",
            stderr(&output)
        );
        let bodies = scripted.bodies.lock().unwrap();
        assert_eq!(bodies.len(), 2, "mode {mode}: write, final text");
        let request = request_json(&bodies[0]);
        let names = tool_names(&request);
        assert!(
            names.contains(&"workspace_read"),
            "mode {mode}: reads stay advertised: {names:?}"
        );
        assert!(
            !names.contains(&"workspace_write"),
            "mode {mode}: no write tool without a prompt surface: {names:?}"
        );
        assert!(
            !names.contains(&"workspace_edit"),
            "mode {mode}: no edit tool without a prompt surface: {names:?}"
        );
        let error = last_tool_result(&bodies[1]);
        assert!(
            error.contains("unknown tool 'workspace_write'"),
            "mode {mode}: the loop answers the unadvertised call: {error}"
        );
        drop(bodies);
        assert!(
            !env.project.join("notes/leak.md").exists(),
            "mode {mode}: nothing was created"
        );
        assert!(
            !std::fs::read_to_string(env.project.join("src/lib.rs"))
                .unwrap()
                .is_empty(),
            "mode {mode}: the seeded file is untouched"
        );
        let _ = std::fs::remove_dir_all(&env.root);
    }
}

#[test]
fn no_database_profile_is_needed() {
    let env = env("no-profile");
    // The whole coding flow runs with no connections file anywhere: none in
    // the config home, none in the project, none on the command line.
    assert!(!env.root.join("connections.toml").exists());
    assert!(!env.project.join("connections.toml").exists());
    let file = env.project.join("src/lib.rs");
    std::fs::write(&file, FIXTURE).unwrap();
    let scripted = script(read_edit_rounds(None));
    let output = run(
        &env,
        &scripted,
        "bypass",
        "fix the add function in src/lib.rs\n",
    );
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "the session starts and completes with no database profile; stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        EDITED,
        "the full read-edit-report flow ran with no database; stdout:\n{out}"
    );
    drop(scripted);
    let _ = std::fs::remove_dir_all(&env.root);
}
