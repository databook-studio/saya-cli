//! Tests for the workspace search tools: `workspace_list`, `glob`, and
//! `grep`. The defining invariants: with no workspace attached each tool
//! denies with a typed error; a path outside the workspace surfaces the
//! harness's own containment reason; every bound fails as a typed bounds
//! error rather than truncating quietly (a truncated "no matches" reads as
//! proof of absence); and a symlink — wherever it points — is neither
//! followed nor matched by the search.

use std::{fs, path::PathBuf, sync::Arc};

use super::*;
use saya_agent::{LocalStateEffect, ToolError, ToolExecutor};
use saya_harness::workspace::Workspace;

use super::database_tools::{
    WORKSPACE_GLOB_MAX_MATCHES, WORKSPACE_GLOB_MAX_VISITED, WORKSPACE_GREP_MAX_FILE_BYTES,
    WORKSPACE_GREP_MAX_LINE_BYTES, WORKSPACE_GREP_MAX_MATCHES, WORKSPACE_GREP_MAX_VISITED,
    WORKSPACE_LIST_MAX_ENTRIES,
};

/// A sandbox workspace under the OS temp dir, removed on drop. The root sits
/// one level down (`outer/ws`) so a `..` escape and an outside symlink target
/// have real places to name.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wssearch-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }

    /// Plants many files directly (no per-file fsync): only the bulk-bound
    /// tests need more files than the contained write is comfortable with.
    fn plant(&self, names: impl Iterator<Item = String>, bytes: &[u8]) {
        for name in names {
            fs::write(self.ws.root().join(name), bytes).expect("plant must succeed");
        }
    }

    /// Database tools with the sandbox workspace attached and no connections —
    /// a workspace-only run has no selected profile.
    fn tools(&self) -> DatabaseTools {
        DatabaseTools::with_registry(
            crate::connection::ConnectionRegistry::new("primary"),
            100,
            true,
            None,
        )
        .with_workspace(Some(Arc::new(self.ws.clone())))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

// ---------------------------------------------------------------- list

#[tokio::test]
async fn workspace_list_returns_sorted_entries_with_kinds_and_sizes() {
    let sandbox = Sandbox::new("list-content");
    sandbox
        .ws
        .write("notes/summary.md", b"hello workspace")
        .expect("contained write must succeed");
    sandbox
        .ws
        .write("top.txt", b"abcde")
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let root = tools
        .execute("workspace_list", serde_json::json!({}))
        .await
        .expect("listing the root must succeed");
    assert_eq!(root["path"], "");
    let entries = root["entries"].as_array().expect("entries array");
    let names: Vec<&str> = entries
        .iter()
        .map(|entry| entry["name"].as_str().expect("name"))
        .collect();
    assert_eq!(
        names,
        vec!["notes", "top.txt"],
        "entries are sorted by name"
    );
    assert_eq!(entries[0]["kind"], "dir");
    assert_eq!(entries[1]["kind"], "file");
    assert_eq!(entries[1]["size"].as_u64(), Some(5));
    let notes = tools
        .execute("workspace_list", serde_json::json!({"path": "notes"}))
        .await
        .expect("listing a subdirectory must succeed");
    assert_eq!(notes["path"], "notes");
    assert_eq!(notes["entries"][0]["name"], "summary.md");
    assert_eq!(notes["entries"][0]["kind"], "file");
    assert_eq!(notes["entries"][0]["size"].as_u64(), Some(15));
}

/// No workspace attached (every current caller, until the run engine opens
/// one): the tool denies with a typed error instead of pretending to list.
#[tokio::test]
async fn workspace_list_denies_when_no_workspace_is_attached() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("workspace_list", serde_json::json!({}))
        .await
        .expect_err("no workspace, no listing");
    assert_eq!(error, ToolError::WorkspaceUnavailable);
}

/// The escape the containment layer exists for: a `..` path that rises above
/// the root is refused as a typed `ToolError` carrying the harness's own
/// reason — readable by the model, not a panic and not a silent empty list.
#[tokio::test]
async fn workspace_list_surfaces_a_path_escape_as_a_typed_error() {
    let sandbox = Sandbox::new("list-escape");
    fs::write(sandbox.outer.join("outside.txt"), b"secret").expect("plant must succeed");
    let tools = sandbox.tools();
    for path in ["../outside.txt", "/etc/passwd"] {
        let error = tools
            .execute("workspace_list", serde_json::json!({"path": path}))
            .await
            .expect_err("an escaping path must be refused, not listed");
        match error {
            ToolError::Workspace(message) => {
                assert!(
                    message.contains("escapes the run workspace"),
                    "the refusal must name the containment failure: {message}"
                );
                assert!(
                    message.contains(path),
                    "the refusal must name the refused path: {message}"
                );
            }
            other => panic!("expected a typed workspace error, got: {other}"),
        }
    }
}

/// A directory holding more than the entry bound is a typed bounds error, not
/// a quietly truncated list.
#[tokio::test]
async fn workspace_list_reports_an_entry_count_overrun_as_a_typed_error() {
    let sandbox = Sandbox::new("list-bound");
    sandbox.plant(
        (0..WORKSPACE_LIST_MAX_ENTRIES + 1).map(|index| format!("f{index}.txt")),
        b"",
    );
    let tools = sandbox.tools();
    let error = tools
        .execute("workspace_list", serde_json::json!({}))
        .await
        .expect_err("an over-bound directory must be refused, not truncated");
    match error {
        ToolError::Workspace(message) => {
            assert!(
                message.contains("bound exceeded"),
                "the refusal must name the bounds failure: {message}"
            );
        }
        other => panic!("expected a typed workspace error, got: {other}"),
    }
}

#[tokio::test]
async fn workspace_list_rejects_a_non_string_path_and_unknown_arguments() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("workspace_list", serde_json::json!({"path": 7}))
        .await
        .expect_err("a non-string path must be rejected at validation");
    assert_eq!(error, ToolError::PathNotString);
    let error = tools
        .execute(
            "workspace_list",
            serde_json::json!({"path": "notes", "sql": "SELECT 1"}),
        )
        .await
        .expect_err("unknown arguments must be rejected");
    assert_eq!(error, ToolError::UnsupportedProperty);
}

// ---------------------------------------------------------------- glob

/// Runs one `glob` call and returns the matched paths.
async fn glob_paths(tools: &DatabaseTools, pattern: &str) -> Vec<String> {
    let result = tools
        .execute("glob", serde_json::json!({"pattern": pattern}))
        .await
        .expect("a contained glob must succeed");
    result["matches"]
        .as_array()
        .expect("matches array")
        .iter()
        .map(|value| value.as_str().expect("path string").to_string())
        .collect()
}

#[tokio::test]
async fn glob_matches_contained_paths_with_glob_semantics() {
    let sandbox = Sandbox::new("glob-semantics");
    for path in ["a.md", "notes/b.md", "notes/deep/c.md", "code/x.rs"] {
        sandbox
            .ws
            .write(path, b"content")
            .expect("contained write must succeed");
    }
    let tools = sandbox.tools();
    assert_eq!(
        glob_paths(&tools, "**/*.md").await,
        vec!["a.md", "notes/b.md", "notes/deep/c.md"],
        "`**` spans segments and also matches zero of them"
    );
    assert_eq!(
        glob_paths(&tools, "*.md").await,
        vec!["a.md"],
        "`*` stays inside one segment"
    );
    assert_eq!(glob_paths(&tools, "notes/*.md").await, vec!["notes/b.md"]);
    assert_eq!(
        glob_paths(&tools, "notes/**").await,
        vec!["notes", "notes/b.md", "notes/deep", "notes/deep/c.md"],
        "a trailing `**` matches the directory and everything real under it"
    );
    assert_eq!(
        glob_paths(&tools, "/etc/**").await,
        Vec::<String>::new(),
        "an absolute pattern can never match: only walk-contained paths are candidates"
    );
    assert_eq!(
        glob_paths(&tools, "../outside/**").await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn glob_denies_when_no_workspace_is_attached() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("glob", serde_json::json!({"pattern": "**/*.md"}))
        .await
        .expect_err("no workspace, no glob");
    assert_eq!(error, ToolError::WorkspaceUnavailable);
}

#[tokio::test]
async fn glob_reports_a_match_count_overrun_as_a_typed_error() {
    let sandbox = Sandbox::new("glob-match-bound");
    sandbox.plant(
        (0..WORKSPACE_GLOB_MAX_MATCHES + 1).map(|index| format!("f{index}.log")),
        b"",
    );
    let tools = sandbox.tools();
    let error = tools
        .execute("glob", serde_json::json!({"pattern": "*.log"}))
        .await
        .expect_err("an over-bound match list must be refused, not truncated");
    match error {
        ToolError::Workspace(message) => {
            assert!(
                message.contains("bound exceeded"),
                "the refusal must name the bounds failure: {message}"
            );
        }
        other => panic!("expected a typed workspace error, got: {other}"),
    }
}

/// Both search tools share one visited-entries discipline: walking past the
/// visited bound is a typed error for each of them, never a quiet stop.
#[tokio::test]
async fn search_reports_a_visited_count_overrun_as_a_typed_error() {
    let sandbox = Sandbox::new("search-visited-bound");
    let total = WORKSPACE_GLOB_MAX_VISITED.max(WORKSPACE_GREP_MAX_VISITED) + 1;
    sandbox.plant((0..total).map(|index| format!("f{index}.txt")), b"");
    let tools = sandbox.tools();
    let calls = [
        ("glob", serde_json::json!({"pattern": "*.txt"})),
        (
            "grep",
            serde_json::json!({"pattern": "needle", "case_insensitive": false}),
        ),
    ];
    for (name, arguments) in calls {
        let error = tools
            .execute(name, arguments)
            .await
            .expect_err("an over-bound walk must be refused, not truncated");
        match error {
            ToolError::Workspace(message) => {
                assert!(
                    message.contains("bound exceeded"),
                    "the refusal must name the bounds failure: {message}"
                );
            }
            other => panic!("expected a typed workspace error, got: {other}"),
        }
    }
}

/// A symlink into the outside world is a decoy twice over: `glob` must not
/// match it by name (`decoy.md` matches `*.md`), must not descend into a
/// symlinked directory, and `grep` must never read through either.
#[cfg(unix)]
#[tokio::test]
async fn search_neither_follows_nor_matches_an_outside_symlink() {
    use std::os::unix::fs::symlink;

    let sandbox = Sandbox::new("symlink");
    fs::write(sandbox.outer.join("outside.md"), b"outside needle")
        .expect("outside plant must succeed");
    fs::create_dir_all(sandbox.outer.join("outside_dir")).expect("outside dir must create");
    fs::write(
        sandbox.outer.join("outside_dir").join("inside.md"),
        b"inside needle",
    )
    .expect("outside plant must succeed");
    symlink("../outside.md", sandbox.ws.root().join("decoy.md"))
        .expect("decoy symlink must create");
    symlink("../outside_dir", sandbox.ws.root().join("linkdir"))
        .expect("linkdir symlink must create");
    sandbox
        .ws
        .write("real/inner.md", b"real needle")
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute("glob", serde_json::json!({"pattern": "**/*.md"}))
        .await
        .expect("a contained glob must succeed");
    let paths: Vec<String> = result["matches"]
        .as_array()
        .expect("matches array")
        .iter()
        .map(|value| value.as_str().expect("path string").to_string())
        .collect();
    assert_eq!(paths, vec!["real/inner.md"]);
    let result = tools
        .execute("grep", serde_json::json!({"pattern": "needle"}))
        .await
        .expect("a contained grep must succeed");
    let hits = result["matches"].as_array().expect("matches array");
    assert_eq!(hits.len(), 1, "only the real file is searched");
    assert_eq!(hits[0]["path"], "real/inner.md");
    assert_eq!(result["files_scanned"].as_u64(), Some(1));
    assert_eq!(result["files_skipped"].as_u64(), Some(0));
}

#[tokio::test]
async fn glob_rejects_a_non_string_pattern_and_unknown_arguments() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("glob", serde_json::json!({"pattern": 7}))
        .await
        .expect_err("a non-string pattern must be rejected at validation");
    assert_eq!(error, ToolError::PatternNotString);
    let error = tools
        .execute(
            "glob",
            serde_json::json!({"pattern": "*.md", "path": "notes"}),
        )
        .await
        .expect_err("unknown arguments must be rejected");
    assert_eq!(error, ToolError::UnsupportedProperty);
}

// ---------------------------------------------------------------- grep

#[tokio::test]
async fn grep_reports_literal_hits_with_one_based_line_numbers() {
    let sandbox = Sandbox::new("grep-content");
    sandbox
        .ws
        .write("log.txt", b"one\ntwo\nneedle\nfour")
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute("grep", serde_json::json!({"pattern": "needle"}))
        .await
        .expect("a contained grep must succeed");
    assert_eq!(result["files_scanned"].as_u64(), Some(1));
    assert_eq!(result["files_skipped"].as_u64(), Some(0));
    let hits = result["matches"].as_array().expect("matches array");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["path"], "log.txt");
    assert_eq!(
        hits[0]["line"].as_u64(),
        Some(3),
        "line numbers are 1-based"
    );
    assert_eq!(hits[0]["text"], "needle");
    assert_eq!(hits[0]["truncated"], serde_json::Value::Bool(false));
}

#[tokio::test]
async fn grep_honours_the_case_insensitive_flag() {
    let sandbox = Sandbox::new("grep-case");
    sandbox
        .ws
        .write("log.txt", b"Hello World\nsecond")
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "grep",
            serde_json::json!({"pattern": "hello world", "case_insensitive": false}),
        )
        .await
        .expect("a contained grep must succeed");
    assert_eq!(result["matches"].as_array().expect("matches").len(), 0);
    let result = tools
        .execute(
            "grep",
            serde_json::json!({"pattern": "hello world", "case_insensitive": true}),
        )
        .await
        .expect("a contained grep must succeed");
    let hits = result["matches"].as_array().expect("matches array");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["line"].as_u64(), Some(1));
}

/// A file bigger than the per-file search horizon is skipped whole, never
/// half-searched: a hit list over a prefix would read as full coverage. A
/// non-UTF-8 file is skipped rather than served as mojibake. Both land in
/// `files_skipped`, so a miss is never mistaken for proof of absence.
#[tokio::test]
async fn grep_skips_oversized_and_non_utf8_files_and_counts_the_skips() {
    let sandbox = Sandbox::new("grep-skip");
    let mut oversized = vec![b'a'; WORKSPACE_GREP_MAX_FILE_BYTES as usize];
    oversized.extend_from_slice(b"needle");
    sandbox
        .ws
        .write("big.txt", &oversized)
        .expect("contained write must succeed");
    sandbox
        .ws
        .write("bin.dat", b"\xff\xfe\xfd")
        .expect("contained write must succeed");
    sandbox
        .ws
        .write("small.txt", b"needle here")
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute("grep", serde_json::json!({"pattern": "needle"}))
        .await
        .expect("a contained grep must succeed");
    let hits = result["matches"].as_array().expect("matches array");
    assert_eq!(hits.len(), 1, "only the fully readable file is searched");
    assert_eq!(hits[0]["path"], "small.txt");
    assert_eq!(result["files_scanned"].as_u64(), Some(1));
    assert_eq!(result["files_skipped"].as_u64(), Some(2));
}

/// One enormous line cannot blow the context: the reported text is capped at
/// exactly the line bound and the cap is reported on the hit.
#[tokio::test]
async fn grep_bounds_the_reported_line_itself() {
    let sandbox = Sandbox::new("grep-line-bound");
    let mut line = b"needle".to_vec();
    line.extend(std::iter::repeat_n(b'a', 20_000));
    sandbox
        .ws
        .write("wide.txt", &line)
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute("grep", serde_json::json!({"pattern": "needle"}))
        .await
        .expect("a contained grep must succeed");
    let hits = result["matches"].as_array().expect("matches array");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["line"].as_u64(), Some(1));
    assert_eq!(
        hits[0]["text"].as_str().expect("text").len(),
        WORKSPACE_GREP_MAX_LINE_BYTES
    );
    assert_eq!(hits[0]["truncated"], serde_json::Value::Bool(true));
}

#[tokio::test]
async fn grep_reports_a_match_count_overrun_as_a_typed_error() {
    let sandbox = Sandbox::new("grep-match-bound");
    sandbox.plant(
        (0..WORKSPACE_GREP_MAX_MATCHES + 1).map(|index| format!("f{index}.txt")),
        b"needle",
    );
    let tools = sandbox.tools();
    let error = tools
        .execute("grep", serde_json::json!({"pattern": "needle"}))
        .await
        .expect_err("an over-bound match list must be refused, not truncated");
    match error {
        ToolError::Workspace(message) => {
            assert!(
                message.contains("bound exceeded"),
                "the refusal must name the bounds failure: {message}"
            );
        }
        other => panic!("expected a typed workspace error, got: {other}"),
    }
}

#[tokio::test]
async fn grep_denies_when_no_workspace_is_attached() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("grep", serde_json::json!({"pattern": "needle"}))
        .await
        .expect_err("no workspace, no grep");
    assert_eq!(error, ToolError::WorkspaceUnavailable);
}

#[tokio::test]
async fn grep_rejects_a_non_string_pattern_and_a_non_boolean_flag() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("grep", serde_json::json!({"pattern": 7}))
        .await
        .expect_err("a non-string pattern must be rejected at validation");
    assert_eq!(error, ToolError::PatternNotString);
    let error = tools
        .execute(
            "grep",
            serde_json::json!({"pattern": "needle", "case_insensitive": "yes"}),
        )
        .await
        .expect_err("a non-boolean flag must be rejected at validation");
    assert_eq!(error, ToolError::CaseInsensitiveNotBool);
    let error = tools
        .execute("grep", serde_json::json!({"pattern": "needle", "extra": 1}))
        .await
        .expect_err("unknown arguments must be rejected");
    assert_eq!(error, ToolError::UnsupportedProperty);
}

// ---------------------------------------------------------- definitions

/// All three are advertised regardless of the data-sharing gate (they touch
/// no database data) and are read-shaped: read-only approval permits them,
/// they declare `LocalStateEffect::Read`, and each states its own completion —
/// the generic read-only wording says "database", which these are not.
#[test]
fn workspace_search_definitions_are_read_shaped_and_always_advertised() {
    let tools = DatabaseTools::definitions(false, false, false, false);
    let completions = [
        ("workspace_list", "workspace directory listed"),
        ("glob", "workspace paths matched"),
        ("grep", "workspace text searched"),
    ];
    for (name, completion) in completions {
        let tool = tools
            .iter()
            .find(|tool| tool.name == name)
            .expect("each workspace search tool is advertised even with the data gate closed");
        assert!(tool.read_only);
        assert!(!tool.effect.database_data);
        assert!(!tool.effect.external_side_effect);
        assert!(!tool.effect.requires_approval);
        assert_eq!(tool.effect.local_state, LocalStateEffect::Read);
        assert_eq!(
            tool.parameters["additionalProperties"],
            serde_json::Value::Bool(false)
        );
        assert_eq!(tool.completion.as_deref(), Some(completion));
    }
    let required = |name: &str| -> Vec<String> {
        tools
            .iter()
            .find(|tool| tool.name == name)
            .expect("definition exists")
            .parameters["required"]
            .as_array()
            .expect("required list")
            .iter()
            .map(|value| value.as_str().expect("name").to_string())
            .collect()
    };
    assert_eq!(required("workspace_list"), Vec::<String>::new());
    assert_eq!(required("glob"), vec!["pattern".to_string()]);
    assert_eq!(required("grep"), vec!["pattern".to_string()]);
}
