fn append_workspace_read_tools(tools: &mut Vec<ToolDefinition>) {
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
        concurrency: ToolConcurrency::Serial,
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
        concurrency: ToolConcurrency::Serial,
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
        concurrency: ToolConcurrency::Serial,
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
        concurrency: ToolConcurrency::Serial,
        completion: Some("workspace text searched".into()),
    });
}
