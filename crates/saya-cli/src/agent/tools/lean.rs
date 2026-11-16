//! The lean, model-facing text and advertised parameter schemas for the seven
//! coding tools every surface shares.
//!
//! One copy, because a surface that advertises a tool must not be able to
//! describe it differently from the executor that accepts it. The executors
//! keep accepting every parameter they ever accepted — `edit`'s
//! `offset`/`chunk`/`edits`/`expected_size`, `shell`'s `sandbox`/`workdir`,
//! `checklist`'s `note` — but the advertisement states only what a model needs
//! to start, and the refusal it earns names the accepted form when it reaches
//! for an edge. `grep`'s `type`/`multiline`/`head_limit` and `glob`'s `type`
//! are advertised because the engine already bounds them.

use serde_json::{Value, json};

pub(crate) const READ: &str = "Read a file. `path` is relative to the workspace root (an absolute path under it is accepted). Optional `start_line` and `line_count` (up to 2000) select lines, numbered as `N→text` (the prefix is not in the file). Returns `content` and `digest`; `truncated` is set when content was cut.";

pub(crate) const WRITE: &str = "Write a whole file. Creates it, or replaces a file you read earlier in this run (else pass its `expected_digest`). Returns `digest` and a short head of the written file.";

pub(crate) const EDIT: &str = "Replace `old_text` with `new_text` in a file. `old_text` must match exactly once unless `replace_all` is true; never include the `N→` prefix. Pass `edits` (a list of `old_text`/`new_text` pairs) to change several places in one call. Pass `expected_digest` (the `digest` from your last write or edit of the file) unless you read it earlier in this run. Returns the new `digest` and the changed lines with line numbers.";

pub(crate) const GLOB: &str = "Find files by pattern such as `**/*.ts`, newest first. Optional `path` limits the search to a directory (relative to the root; an absolute path under it is accepted).";

pub(crate) const GREP: &str = "Search file contents. `pattern` is a literal substring unless `regex` is true. Optional `path` scope (relative to the root; an absolute path under it is accepted), `include` glob, `case_insensitive`, `context` lines, and `output_mode` (`content`, `files_with_matches`, `count`).";

pub(crate) const SHELL: &str = "Run a /bin/sh command in the working checkout. Join related commands in one call with `&&` or `;`. Optional `timeout` in milliseconds (default 120000, max 600000). Output is capped at 30,000 characters per call. Keep searches (grep/find/rg) inside the workspace unless the task names another path. Prefer `read`, `grep`, and `glob` for inspection. Run the narrowest relevant test first; run a full suite only when asked or the change is cross-cutting. A long result returns its head and tail with the exit code; `truncated` marks a dropped middle.";

pub(crate) const SHELL_SESSION: &str = " May ask the user first.";

pub(crate) const CHECKLIST: &str = "Replace your whole task list. Set it once for multi-step work and update it only when a step's status changes. Each task has `title` and `status` (`pending`, `in_progress`, `done`).";

pub(crate) fn read_parameters() -> Value {
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
}

pub(crate) fn write_parameters() -> Value {
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
}

pub(crate) fn edit_parameters() -> Value {
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
}

pub(crate) fn glob_parameters() -> Value {
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
}

pub(crate) fn grep_parameters() -> Value {
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
}

pub(crate) fn shell_parameters() -> Value {
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
}

pub(crate) fn checklist_parameters() -> Value {
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
}
