use saya_agent::{LocalStateEffect, ToolDefinition, ToolEffect, ToolError};

use super::DatabaseTools;

impl DatabaseTools {
    /// Returns available database tool definitions. Contract read tools are
    /// appended only when a state store is present **and** database context is
    /// allowed; when the privacy gate forbids database context they are hidden
    /// rather than advertised as always-empty, matching the read-tool
    /// precedent. `permit_candidate_writes` gates nothing here: no tool
    /// declares `WriteCandidate` — proposals are extracted post-turn from the
    /// bounded turn record — so the flag is accepted only to keep the call
    /// sites unchanged. `permit_workspace_writes` gates `workspace_write` the
    /// hidden-not-advertised way: a write tool the model can see but never use
    /// wastes context and invites retries, so it is omitted until the run was
    /// constructed with workspace writes permitted.
    pub(crate) fn definitions(
        allow_query_data: bool,
        has_state_store: bool,
        permit_candidate_writes: bool,
        permit_workspace_writes: bool,
    ) -> Vec<ToolDefinition> {
        let connection_prop = serde_json::json!({
            "type": "string",
            "description": "Optional. Name of the database connection to target; defaults to the primary. Available connections and their dialects are listed in the system context."
        });

        let mut tools = vec![ToolDefinition {
            name: "schema_discovery".into(),
            description: "Inspect the selected database schema without changing data.".into(),
            read_only: true,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "connection": connection_prop
                },
                "additionalProperties": false
            }),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            },
            completion: None,
        }];
        // The workspace read is unconditional: it touches no database data, so
        // the privacy gate does not hide it. When no workspace is attached —
        // every path until the run engine passes one in — dispatch denies with
        // a typed error rather than the definition advertising a dead tool
        // silently succeeding. It states its own completion: the generic
        // read-only wording says "database", which a workspace file read is
        // not.
        tools.push(ToolDefinition {
            name: "workspace_read".into(),
            description: "Read one file from this run's workspace — the contained \
                directory holding this run's files. Pass `path` relative to the \
                workspace root; absolute paths, `..` escapes, and symlinks are \
                refused. Returns `content`, the file's full `size` in bytes, \
                `truncated`, and `digest` — the sha256 of the file's whole \
                bytes, so a truncated read still names the state an \
                `expected_digest` edit precondition can state. Read `truncated` \
                first: when it is true, `content` is only a prefix capped at \
                the read bound — use `size` to judge how much was cut, and \
                never present capped content as the whole file."
                .into(),
            read_only: true,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path relative to the workspace root."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::Read,
            },
            completion: Some("workspace file read".into()),
        });
        // The workspace search tools are unconditional like `workspace_read`:
        // they touch no database data, so the privacy gate does not hide
        // them, and each states its own completion — the generic read-only
        // wording says "database", which a workspace search is not.
        tools.push(ToolDefinition {
            name: "workspace_list".into(),
            description: "List one directory of this run's workspace — the contained \
                directory holding this run's files. Pass `path` relative to the workspace \
                root; omit it to list the root itself. Returns sorted `entries`, each with \
                its `name`, `kind` (`file`, `dir`, `symlink`, or `other`), and `size` in \
                bytes. Symlinks are reported as entries and never followed, and a \
                directory holding more entries than the bound is refused rather than \
                silently shortened."
                .into(),
            read_only: true,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path relative to the workspace root; \
                            omit to list the root itself."
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::Read,
            },
            completion: Some("workspace directory listed".into()),
        });
        tools.push(ToolDefinition {
            name: "glob".into(),
            description: "Find paths in this run's workspace matching a glob pattern, \
                relative to the workspace root. `**` spans directory segments (and matches \
                zero of them), `*` stays inside one segment; directories match too, so \
                `notes/**` returns the `notes` directory itself plus everything under it. \
                Returns sorted `matches` — real, contained paths only: absolute patterns \
                and `..` prefixes can never match, and symlinks are neither matched nor \
                descended into. If the walk or the match list would exceed a bound, the \
                call fails with the reason instead of returning a shortened list."
                .into(),
            read_only: true,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern relative to the workspace root, \
                            e.g. `**/*.md` or `notes/*.txt`."
                    }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::Read,
            },
            completion: Some("workspace paths matched".into()),
        });
        tools.push(ToolDefinition {
            name: "grep".into(),
            description: "Search this run's workspace files for a literal substring — no \
                regex. Returns `matches`, each with the file `path`, 1-based `line`, the \
                line's `text` (capped per line, with `truncated` when capped), plus two \
                coverage counts you must read before trusting a miss: `files_scanned` is \
                how many files were actually read and searched, and `files_skipped` is how \
                many were never searched (too large for the per-file bound, or not valid \
                UTF-8). `files_scanned == 0` with `files_skipped > 0` is NOT evidence the \
                text is absent — narrow the search and retry."
                .into(),
            read_only: true,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Literal substring to search for, matched against \
                            each file's lines."
                    },
                    "case_insensitive": {
                        "type": "boolean",
                        "description": "Match ignoring letter case. Defaults to false."
                    }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::Read,
            },
            completion: Some("workspace text searched".into()),
        });
        // The workspace write is the first model-facing tool that writes
        // anything, so it declares `WriteWorkspace` honestly: that declaration
        // is what the loop's fail-closed gate keys on, and the scope approval
        // plus the permit are the gate — there is deliberately no per-call
        // prompt (D7). It is pushed only when workspace writes are permitted:
        // advertised-but-unusable would waste context and invite retries, so
        // the definition is hidden, not merely dead (D15 admits
        // `workspace_edit` beside it — anchored replace plus offset-checked
        // append over one atomic contained operation).
        if permit_workspace_writes {
            tools.push(ToolDefinition {
                name: "workspace_edit".into(),
                description: "Replace one anchored string in this run's workspace — the \
                    contained directory holding this run's files. Pass `path` relative \
                    to the workspace root, `old_text` as the exact text to find, and \
                    `new_text` as its replacement; the anchor must match exactly once \
                    — zero matches or multiple matches refuse and change nothing, \
                    never \"first wins\". Or append one chunk: pass `offset` (the \
                    file size you measured) and `chunk`; the offset must equal the \
                    current size — a mismatch refuses with the current size and \
                    digest so you resume from there — and offset 0 on an absent \
                    path creates the file. An empty anchor is rejected, anchor and \
                    replacement and chunk over the bound refuse whole, and a \
                    non-UTF-8 target is refused. Pass `expected_size` and/or \
                    `expected_digest` from a fresh read to guard a moved anchor: \
                    a mismatch refuses with no write. Concurrent writers are \
                    last-writer-wins unless `expected_*` is supplied. This tool \
                    never resumes a stopped response on its own: after an \
                    output-token cap, resume by appending from the reported size \
                    and digest. For small whole-file writes use `workspace_write` \
                    instead. Returns the `path`, `size`, \
                    and `digest` (the replace variant also reports \
                    `bytes_replaced` and `bytes_written`)."
                    .into(),
                read_only: false,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File path relative to the workspace root."
                        },
                        "old_text": {
                            "type": "string",
                            "description": "The exact text to find; must occur exactly once."
                        },
                        "new_text": {
                            "type": "string",
                            "description": "The replacement text stored in place of the anchor."
                        },
                        "offset": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "Append only. The file size in bytes the chunk continues from; must equal the current size."
                        },
                        "chunk": {
                            "type": "string",
                            "description": "Append only. The bytes to store at the end of the file."
                        },
                        "expected_size": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "Optional. The file size in bytes the anchor was measured against; a mismatch refuses with no write."
                        },
                        "expected_digest": {
                            "type": "string",
                            "description": "Optional. The file's sha256 hex digest the anchor was measured against; a mismatch refuses with no write."
                        }
                    },
                    "required": ["path", "old_text", "new_text"],
                    "additionalProperties": false
                }),
                effect: ToolEffect {
                    database_data: false,
                    external_side_effect: false,
                    requires_approval: false,
                    local_state: LocalStateEffect::WriteWorkspace,
                },
                completion: Some("workspace file edited".into()),
            });
            tools.push(ToolDefinition {
                name: "workspace_write".into(),
                description: "Write one file into this run's workspace — the contained \
                    directory holding this run's files. Pass `path` relative to the workspace \
                    root and `content` as the full text to store; the file is written \
                    atomically — replaced whole or not at all, never partially. Absolute \
                    paths, `..` escapes, and symlinks are refused, content over the write \
                    bound is refused whole, and existing files are replaced by the new \
                    content. Returns the `path` and `bytes_written`."
                    .into(),
                read_only: false,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File path relative to the workspace root."
                        },
                        "content": {
                            "type": "string",
                            "description": "The full text to store in the file, replacing \
                                any previous content."
                        }
                    },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }),
                effect: ToolEffect {
                    database_data: false,
                    external_side_effect: false,
                    requires_approval: false,
                    local_state: LocalStateEffect::WriteWorkspace,
                },
                completion: Some("workspace file written".into()),
            });
        }
        if allow_query_data {
            tools.push(ToolDefinition {
                name: "bounded_sql_query".into(),
                description: "Run one bounded read-only SQL query against the selected database."
                    .into(),
                read_only: true,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "connection": connection_prop.clone(),
                        "sql": {
                            "type": "string"
                        }
                    },
                    "required": ["sql"],
                    "additionalProperties": false
                }),
                effect: ToolEffect {
                    database_data: true,
                    external_side_effect: false,
                    requires_approval: true,
                    local_state: LocalStateEffect::None,
                },
                completion: None,
            });
            tools.push(ToolDefinition {
                name: "bounded_sql_query_all".into(),
                description: "Run one bounded read-only SQL query against EVERY connected \
                    database at once and return the per-database results. Use this when the \
                    same question should be answered across all connected databases; each \
                    database runs independently, so a failure on one (e.g. a dialect \
                    mismatch) is reported alongside the successes rather than aborting the \
                    rest. Do not pass a `connection` argument."
                    .into(),
                read_only: true,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "sql": {
                            "type": "string"
                        }
                    },
                    "required": ["sql"],
                    "additionalProperties": false
                }),
                effect: ToolEffect {
                    database_data: true,
                    external_side_effect: false,
                    requires_approval: true,
                    local_state: LocalStateEffect::None,
                },
                completion: None,
            });
            tools.push(ToolDefinition {
                name: "result_shape".into(),
                description: "Run one bounded read-only SQL query and return its SHAPE only — \
                    the row count, whether the row cap was hit, and the column names with a \
                    type label — and never any row values. Use this instead of \
                    bounded_sql_query when you only need to know whether a query worked, \
                    roughly how many rows it returned, and what columns came back; it costs \
                    far less context than fetching the rows. Read `truncated` first: when it \
                    is true, `row_count` is a floor and not the real total, so a capped count \
                    read as the true count leads to a false conclusion."
                    .into(),
                read_only: true,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "connection": connection_prop.clone(),
                        "sql": { "type": "string" }
                    },
                    "required": ["sql"],
                    "additionalProperties": false
                }),
                effect: ToolEffect {
                    database_data: false,
                    external_side_effect: false,
                    requires_approval: true,
                    local_state: LocalStateEffect::None,
                },
                completion: None,
            });
            tools.push(ToolDefinition {
                name: "column_health".into(),
                description: "Run one bounded read-only SQL query and return per-column health \
                    statistics — null count, null percentage, distinct value count, and numeric \
                    zero count — and never any cell value. Use this to run your expression \
                    through the safety path BEFORE trusting it: a 100% null rate after a DATE() \
                    or CAST reveals that the function silently coerced every row (e.g. SQLite's \
                    DATE() returns NULL for '1/1/2021 12:01:36 AM', collapsing every group into \
                    one and turning a per-day average into a lifetime total). Read `truncated` \
                    first: when it is true, the stats are over a capped sample, not the full \
                    result, so a 100% null rate on a sample is still a strong signal but the \
                    counts are floors."
                    .into(),
                read_only: true,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "connection": connection_prop.clone(),
                        "sql": { "type": "string" }
                    },
                    "required": ["sql"],
                    "additionalProperties": false
                }),
                effect: ToolEffect {
                    database_data: false,
                    external_side_effect: false,
                    requires_approval: true,
                    local_state: LocalStateEffect::None,
                },
                completion: None,
            });
            tools.push(ToolDefinition {
                name: "join_check".into(),
                description: "Check whether a JOIN in your query multiplies or drops rows before \
                    you trust a SUM, AVG, or COUNT over it. Builds two COUNT(*) statements — one \
                    over the full join and one over the base table alone — and compares them. \
                    Returns {applicable, joined_rows, base_rows, fanned_out, dropped_rows}. When \
                    the join is on a non-unique key, joined_rows exceeds base_rows and \
                    fanned_out is true, meaning every aggregate over the base table is inflated. \
                    When joined_rows is less than base_rows, the join dropped rows and \
                    dropped_rows is true — just as corrupting. When no sound probe can be built \
                    (no join, no distortable aggregate, subquery in FROM, etc.), returns \
                    applicable: false with a reason — never a guess. Call this WHILE building a \
                    join query, not only after the fact."
                    .into(),
                read_only: true,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "connection": connection_prop.clone(),
                        "sql": { "type": "string" }
                    },
                    "required": ["sql"],
                    "additionalProperties": false
                }),
                effect: ToolEffect {
                    database_data: false,
                    external_side_effect: false,
                    requires_approval: true,
                    local_state: LocalStateEffect::None,
                },
                completion: None,
            });
            tools.push(ToolDefinition {
                name: "render_chart".into(),
                description: "Visualize the results of a SQL query as an interactive chart the user can open \
                    in their browser. Call this whenever the user asks to chart, plot, graph, or visualize \
                    data. Provide the SQL to run and choose the chart_type that best fits the data: `bar` for \
                    comparing categories, `line` or `area` for trends over an ordered/time axis, `pie` or \
                    `doughnut` for a category's share of a total, `scatter` for the relationship between two \
                    numeric columns. Optionally name the x (label) column, the y (value) column(s), and a \
                    title. The chart is written to a file and opened; only the file path is returned."
                    .into(),
                read_only: false,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "connection": connection_prop.clone(),
                        "sql": { "type": "string" },
                        "chart_type": { "type": "string", "enum": ["bar","line","area","pie","doughnut","scatter"] },
                        "x": { "type": "string" },
                        "y": { "type": "array", "items": { "type": "string" } },
                        "title": { "type": "string" }
                    },
                    "required": ["sql", "chart_type"],
                    "additionalProperties": false
                }),
                effect: ToolEffect {
                    database_data: false,
                    external_side_effect: true,
                    requires_approval: true,
                    local_state: LocalStateEffect::None,
                },
                completion: Some("chart written and opened".into()),
            });
            tools.push(ToolDefinition {
                name: "designate_answer".into(),
                description: "Designate the SQL query that answers the user's question — the \
                    statement that produced the answer, never an exploratory probe you ran to \
                    learn the schema or test a guess. Call this exactly once, in your final \
                    message, alongside your prose answer, with the SQL that produced it. This \
                    does not run a query — it records which of the queries you ran is the \
                    answering one. Omit it when no single query answers the question."
                    .into(),
                read_only: true,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "sql": { "type": "string" }
                    },
                    "required": ["sql"],
                    "additionalProperties": false
                }),
                effect: ToolEffect {
                    database_data: false,
                    external_side_effect: false,
                    requires_approval: false,
                    local_state: LocalStateEffect::None,
                },
                completion: None,
            });
        }
        // Contract tools are a sibling concern (see `contract_tools`); they are
        // appended here so the agent receives one flat definition list, matching
        // how this function is assembled for the database tools.
        tools.extend(
            crate::agent::tools::contract_tools::contract_tool_definitions(
                allow_query_data,
                has_state_store,
            ),
        );
        let _ = permit_candidate_writes;
        tools
    }
}

pub(super) fn validate_arguments(
    name: &str,
    arguments: &serde_json::Value,
) -> Result<(), ToolError> {
    let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
    let (allowed, requires_sql) = match name {
        "schema_discovery" => (&["connection"][..], false),
        "workspace_read" => (&["path"][..], false),
        "workspace_list" => (&["path"][..], false),
        "workspace_write" => (&["path", "content"][..], false),
        "workspace_edit" => (
            &[
                "path",
                "old_text",
                "new_text",
                "offset",
                "chunk",
                "expected_size",
                "expected_digest",
            ][..],
            false,
        ),
        "glob" => (&["pattern"][..], false),
        "grep" => (&["pattern", "case_insensitive"][..], false),
        "bounded_sql_query" => (&["connection", "sql"][..], true),
        "bounded_sql_query_all" => (&["sql"][..], true),
        "result_shape" => (&["connection", "sql"][..], true),
        "column_health" => (&["connection", "sql"][..], true),
        "join_check" => (&["connection", "sql"][..], true),
        "render_chart" => (
            &["connection", "sql", "chart_type", "x", "y", "title"][..],
            true,
        ),
        "designate_answer" => (&["sql"][..], true),
        _ => return Err(ToolError::UnsupportedTool),
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(ToolError::UnsupportedProperty);
    }
    if object
        .get("connection")
        .is_some_and(|connection| !connection.is_string())
    {
        return Err(ToolError::ConnectionNotString);
    }
    if requires_sql && !object.get("sql").is_some_and(serde_json::Value::is_string) {
        return Err(ToolError::SqlNotString);
    }
    if name == "workspace_read" && !object.get("path").is_some_and(serde_json::Value::is_string) {
        return Err(ToolError::PathNotString);
    }
    // The `workspace_edit` arguments are a tagged union of two variants:
    // `replace` (`old_text`+`new_text`) and `append` (`offset`+`chunk`).
    // Required strings name their own error, and the optional `expected_*`
    // precondition names its own when present-but-malformed — so the model
    // can fix the right argument. The structural rule is enforced here, not
    // only in the body: mixing the two halves, or half of one, is a typed
    // validation error, never a guess about which variant was meant. No
    // read-side digest, no D15 edit: this slice adds the `append` variant to
    // the existing `replace` shape.
    if name == "workspace_edit" {
        if !object.get("path").is_some_and(serde_json::Value::is_string) {
            return Err(ToolError::PathNotString);
        }
        let has_old = object.contains_key("old_text");
        let has_new = object.contains_key("new_text");
        let has_offset = object.contains_key("offset");
        let has_chunk = object.contains_key("chunk");
        if has_offset || has_chunk {
            // Append: both halves, neither replace half.
            if !object
                .get("offset")
                .is_some_and(|value| !value.is_null() && value.as_u64().is_some())
            {
                return Err(ToolError::OffsetNotUint);
            }
            if !object
                .get("chunk")
                .is_some_and(serde_json::Value::is_string)
            {
                return Err(ToolError::ChunkNotString);
            }
            if has_old || has_new {
                return Err(ToolError::UnsupportedProperty);
            }
        } else {
            if !object
                .get("old_text")
                .is_some_and(serde_json::Value::is_string)
            {
                return Err(ToolError::OldTextNotString);
            }
            if !object
                .get("new_text")
                .is_some_and(serde_json::Value::is_string)
            {
                return Err(ToolError::NewTextNotString);
            }
        }
        if object
            .get("expected_size")
            .is_some_and(|value| !value.is_null() && value.as_u64().is_none())
        {
            return Err(ToolError::ExpectedSizeNotUint);
        }
        if object
            .get("expected_digest")
            .is_some_and(|value| !value.is_null() && !value.is_string())
        {
            return Err(ToolError::ExpectedDigestNotString);
        }
    }
    // Both `workspace_write` arguments are required strings; each names its
    // own typed error so the model can fix the right one.
    if name == "workspace_write" {
        if !object.get("path").is_some_and(serde_json::Value::is_string) {
            return Err(ToolError::PathNotString);
        }
        if !object
            .get("content")
            .is_some_and(serde_json::Value::is_string)
        {
            return Err(ToolError::ContentNotString);
        }
    }
    // `workspace_list`'s path is optional — absent means the root — so only a
    // present non-string is rejected.
    if name == "workspace_list" && object.get("path").is_some_and(|value| !value.is_string()) {
        return Err(ToolError::PathNotString);
    }
    if matches!(name, "glob" | "grep")
        && !object
            .get("pattern")
            .is_some_and(serde_json::Value::is_string)
    {
        return Err(ToolError::PatternNotString);
    }
    if name == "grep"
        && object
            .get("case_insensitive")
            .is_some_and(|value| value.as_bool().is_none())
    {
        return Err(ToolError::CaseInsensitiveNotBool);
    }
    Ok(())
}
