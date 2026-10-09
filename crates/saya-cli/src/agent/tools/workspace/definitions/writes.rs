fn append_workspace_write_tools(tools: &mut Vec<ToolDefinition>, permit_workspace_writes: bool) {
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
                    never \"first wins\". Replacing an existing file requires \
                    `expected_digest`: the `digest` field of a fresh `workspace_read` \
                    or of the previous edit's result — a file that changed since you \
                    observed it refuses with no write, and a same-size change between \
                    the tool's own read and the write is refused by the same guard. \
                    Or append one chunk: pass `offset` (the file size you measured) \
                    and `chunk`; appending to an existing file also requires \
                    `expected_digest` (the digest of a fresh read or of the previous \
                    edit's result), and the offset must equal the current size — a \
                    mismatch refuses with the current size and digest so you resume \
                    from there; offset 0 on an absent path creates the file. An \
                    empty anchor is rejected, anchor and replacement and chunk over \
                    the bound refuse whole, and a non-UTF-8 target is refused. A \
                    mismatched `expected_size` refuses with no write. This tool \
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
                            "description": "Required when replacing or when appending to an existing file; omit only when creating a new file with offset 0."
                        }
                    },
                    "required": ["path"],
                    "oneOf": [
                        { "required": ["old_text", "new_text", "expected_digest"] },
                        { "required": ["offset", "chunk"] }
                    ],
                    "additionalProperties": false
                }),
                effect: ToolEffect {
                    database_data: false,
                    external_side_effect: false,
                    requires_approval: false,
                    local_state: LocalStateEffect::WriteWorkspace,
                },
                concurrency: ToolConcurrency::Serial,
                completion: Some("workspace file edited".into()),
            });
        tools.push(ToolDefinition {
            name: "workspace_write".into(),
            description: "Write one file into this run's workspace — the contained \
                    directory holding this run's files. Pass `path` relative to the workspace \
                    root and `content` as the full text to store. The write is \
                    create-if-absent: it creates the file only when nothing names it yet. To \
                    replace an existing file you must also pass `expected_digest`, the sha256 \
                    hex `digest` you observed from a fresh `workspace_read` or the previous \
                    write/edit result — the write refuses when the file has changed since, so \
                    a concurrent writer's bytes are never clobbered. The file is written \
                    atomically — replaced whole or not at all, never partially. Absolute \
                    paths, `..` escapes, and symlinks are refused, and content over the write \
                    bound is refused whole. Returns the `path`, `bytes_written`, `size`, and \
                    `digest`."
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
                        "description": "The full text to store in the file."
                    },
                    "expected_digest": {
                        "type": "string",
                        "description": "Required to replace an existing file: the sha256 hex \
                            `digest` from a fresh `workspace_read` or from the previous \
                            write/edit result. Omit it only to create a new file; writing \
                            over an existing file without it is refused."
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
            concurrency: ToolConcurrency::Serial,
            completion: Some("workspace file written".into()),
        });
    }
}
