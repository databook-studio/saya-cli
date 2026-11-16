//! The lean, model-facing text and advertised schemas for the seven shared
//! coding tools, pinned exactly: one copy serves the coding root, a helper,
//! the session and a `saya run` step, so a surface cannot drift.
//!
//! The executors keep accepting every parameter they accepted before the
//! advertisement shrank. The acceptance tests below are the proof: the model
//! may still send `edit`'s append form, `shell`'s workdir and sandbox, and
//! `checklist`'s note; `edit`'s `edits`, `grep`'s type, multiline and
//! head_limit and `glob`'s type are advertised and accepted.

use saya_agent::ToolDefinition;
use saya_agent::ToolExecutor;
use serde_json::json;

use super::DatabaseTools;
use super::shell_policy::{self, BackgroundOwner};
use crate::interactive::session::tasks::{SessionTasks, TasksSet, render};

const READ: &str = "Read a file. `path` is relative to the workspace root (an absolute path under it is accepted). Optional `start_line` and `line_count` (up to 2000) select lines, numbered as `N→text` (the prefix is not in the file). Returns `content` and `digest`; `truncated` is set when content was cut.";
const WRITE: &str = "Write a whole file. Creates it, or replaces a file you read earlier in this run (else pass its `expected_digest`). Returns `digest` and a short head of the written file.";
const EDIT: &str = "Replace `old_text` with `new_text` in a file. `old_text` must match exactly once unless `replace_all` is true; never include the `N→` prefix. Pass `edits` (a list of `old_text`/`new_text` pairs) to change several places in one call. Pass `expected_digest` (the `digest` from your last write or edit of the file) unless you read it earlier in this run. Returns the new `digest` and the changed lines with line numbers.";
const GLOB: &str = "Find files by pattern such as `**/*.ts`, newest first. Optional `path` limits the search to a directory (relative to the root; an absolute path under it is accepted).";
const GREP: &str = "Search file contents. `pattern` is a literal substring unless `regex` is true. Optional `path` scope (relative to the root; an absolute path under it is accepted), `include` glob, `case_insensitive`, `context` lines, and `output_mode` (`content`, `files_with_matches`, `count`).";
const SHELL: &str = "Run a /bin/sh command in the working checkout. Join related commands in one call with `&&` or `;`. Optional `timeout` in milliseconds (default 120000, max 600000). Output is capped at 30,000 characters per call. Keep searches (grep/find/rg) inside the workspace unless the task names another path. Prefer `read`, `grep`, and `glob` for inspection. Run the narrowest relevant test first; run a full suite only when asked or the change is cross-cutting. A long result returns its head and tail with the exit code; `truncated` marks a dropped middle.";
const CHECKLIST: &str = "Replace your whole task list. Set it once for multi-step work and update it only when a step's status changes. Each task has `title` and `status` (`pending`, `in_progress`, `done`).";

fn composed(name: &str) -> ToolDefinition {
    DatabaseTools::definitions(true, false, false, true)
        .into_iter()
        .find(|definition| definition.name == name)
        .unwrap_or_else(|| panic!("{name} is composed"))
}

fn assert_description(name: &str, definition: &ToolDefinition, expected: &str) {
    assert_eq!(
        definition.description, expected,
        "{name}'s description is the lean one"
    );
}

#[test]
fn the_shared_workspace_tools_advertise_the_lean_descriptions() {
    for (name, expected) in [
        ("read", READ),
        ("write", WRITE),
        ("edit", EDIT),
        ("glob", GLOB),
        ("grep", GREP),
    ] {
        assert_description(name, &composed(name), expected);
    }
    assert_description("checklist", &render::checklist_definition(), CHECKLIST);
    assert_description("shell", &shell_policy::definition(), SHELL);
}

#[test]
fn a_background_shell_appends_one_short_sentence_that_names_its_ending() {
    for (owner, ending) in [
        (BackgroundOwner::Job, "the job ends"),
        (BackgroundOwner::Helper, "the helper ends"),
        (BackgroundOwner::Session, "the session ends"),
    ] {
        let definition = shell_policy::background_definitions(owner).remove(0);
        let expected = format!(
            "{SHELL} Set `run_in_background` to start it detached and return an id; it runs \
             until it ends, you kill it, or {ending}. A foreground call that hits its \
             `timeout` is moved to the background and answers with its id."
        );
        let expected = if owner == BackgroundOwner::Session {
            format!("{expected} May ask the user first.")
        } else {
            expected
        };
        assert_eq!(definition.description, expected, "{owner:?}");
    }
}

#[test]
fn the_shared_workspace_tools_advertise_the_lean_parameter_schemas() {
    assert_eq!(
        composed("read").parameters,
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {"type": "string"},
                "start_line": {"type": "integer", "minimum": 1},
                "line_count": {"type": "integer", "minimum": 1, "maximum": 2000}
            },
            "required": ["path"]
        })
    );
    assert_eq!(
        composed("write").parameters,
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"},
                "expected_digest": {
                    "type": "string",
                    "description": "The `digest` from your last read/write/edit of this file."
                }
            },
            "required": ["path", "content"]
        })
    );
    assert_eq!(
        composed("edit").parameters,
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {"type": "string"},
                "old_text": {"type": "string"},
                "new_text": {"type": "string"},
                "edits": {
                    "type": "array",
                    "maxItems": 32,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "old_text": {"type": "string"},
                            "new_text": {"type": "string"}
                        },
                        "required": ["old_text", "new_text"]
                    }
                },
                "replace_all": {"type": "boolean"},
                "expected_digest": {"type": "string"}
            },
            "required": ["path"]
        })
    );
    assert_eq!(
        composed("glob").parameters,
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "pattern": {"type": "string"},
                "path": {"type": "string"},
                "type": {
                    "type": "string",
                    "enum": ["file", "dir"],
                    "description": "Keep one entry kind."
                }
            },
            "required": ["pattern"]
        })
    );
    assert_eq!(
        composed("grep").parameters,
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "pattern": {"type": "string"},
                "regex": {"type": "boolean"},
                "path": {"type": "string"},
                "include": {"type": "string"},
                "case_insensitive": {"type": "boolean"},
                "context": {"type": "integer", "minimum": 0, "maximum": 10},
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"]
                },
                "type": {
                    "type": "string",
                    "description": "Only files of this language (e.g. `rust`, `ts`, `py`)."
                },
                "multiline": {
                    "type": "boolean",
                    "description": "Match across line boundaries."
                },
                "head_limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 500,
                    "description": "Serve at most this many entries."
                }
            },
            "required": ["pattern"]
        })
    );
}

#[test]
fn the_shell_and_checklist_advertise_the_lean_parameter_schemas() {
    assert_eq!(
        shell_policy::definition().parameters,
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "command": {"type": "string"},
                "description": {"type": "string"},
                "timeout": {"type": "integer", "minimum": 1, "maximum": 600000}
            },
            "required": ["command"]
        })
    );
    let background = shell_policy::background_definitions(BackgroundOwner::Job).remove(0);
    assert_eq!(
        background.parameters,
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "command": {"type": "string"},
                "description": {"type": "string"},
                "timeout": {"type": "integer", "minimum": 1, "maximum": 600000},
                "run_in_background": {
                    "type": "boolean",
                    "description": "Start the command and return at once with an id; you are told when it exits."
                }
            },
            "required": ["command"]
        })
    );
    assert_eq!(
        render::checklist_definition().parameters,
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "tasks": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "title": {"type": "string"},
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "done"]
                            }
                        },
                        "required": ["title", "status"]
                    }
                }
            },
            "required": ["tasks"]
        })
    );
}

#[test]
fn the_executor_still_accepts_every_edit_form() {
    for arguments in [
        json!({
            "path": "a.txt",
            "offset": 0,
            "chunk": "tail",
            "expected_size": 0,
            "expected_digest": "d"
        }),
        json!({
            "path": "a.txt",
            "edits": [{"old_text": "a", "new_text": "b"}],
            "expected_size": 1,
            "expected_digest": "d"
        }),
        json!({
            "path": "a.txt",
            "old_text": "a",
            "new_text": "b",
            "replace_all": true,
            "expected_size": 1,
            "expected_digest": "d"
        }),
    ] {
        assert!(
            super::workspace::validate_arguments("edit", &arguments).is_ok(),
            "{arguments}"
        );
    }
}

/// The edit shapes share one refusal, so a call that mixes two of them
/// learns the accepted shape from the refusal itself.
#[test]
fn a_mixed_edit_shape_refusal_names_the_one_shape_to_use() {
    for arguments in [
        json!({"path": "a.txt", "offset": 0, "chunk": "tail", "old_text": "a"}),
        json!({
            "path": "a.txt",
            "edits": [{"old_text": "a", "new_text": "b"}],
            "old_text": "a"
        }),
    ] {
        let error = super::workspace::validate_arguments("edit", &arguments)
            .expect_err("two shapes at once refuse");
        let message = error.to_string();
        assert!(
            message.contains("`old_text`/`new_text`")
                && message.contains("`offset`/`chunk`")
                && message.contains("`edits`"),
            "the refusal names every shape: {message}"
        );
    }
}

#[test]
fn the_advertised_grep_filters_are_still_accepted() {
    let arguments = json!({
        "pattern": "needle",
        "type": "rust",
        "multiline": true,
        "head_limit": 5
    });
    assert!(
        super::workspace::validate_arguments("grep", &arguments).is_ok(),
        "{arguments}"
    );
}

#[test]
fn the_executor_still_accepts_the_unadvertised_shell_parameters() {
    let arguments = json!({
        "command": "true",
        "workdir": "sub",
        "sandbox": true,
        "description": "check",
        "timeout": 1000
    });
    assert!(
        shell_policy::validate_arguments(&arguments, false).is_ok(),
        "{arguments}"
    );
}

#[tokio::test]
async fn the_checklist_executor_still_accepts_a_note() {
    let tasks = TasksSet::new(SessionTasks::default());
    let stored = tasks
        .execute(
            "checklist",
            json!({
                "tasks": [
                    {"title": "inspect", "status": "in_progress", "note": "bounded"}
                ]
            }),
        )
        .await
        .expect("the note is still accepted");
    assert_eq!(stored, json!({"tasks": 1}));
}
