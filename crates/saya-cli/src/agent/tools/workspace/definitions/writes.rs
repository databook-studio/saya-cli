fn append_workspace_write_tools(tools: &mut Vec<ToolDefinition>, permit_workspace_writes: bool) {
    // The workspace write is the first model-facing tool that writes
    // anything, so it declares `WriteWorkspace` honestly: that declaration
    // is what the loop's fail-closed gate keys on, and the scope approval
    // plus the permit are the gate — there is deliberately no per-call
    // prompt. It is pushed only when workspace writes are permitted:
    // advertised-but-unusable would waste context and invite retries, so
    // the definition is hidden, not merely dead. `edit` sits beside it —
    // anchored replace plus offset-checked append over one atomic contained
    // operation.
    if permit_workspace_writes {
        tools.push(ToolDefinition {
                name: "edit".into(),
                description: crate::agent::tools::lean::EDIT.into(),
                read_only: false,
                parameters: crate::agent::tools::lean::edit_parameters(),
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
            name: "write".into(),
                description: crate::agent::tools::lean::WRITE.into(),
                read_only: false,
                parameters: crate::agent::tools::lean::write_parameters(),
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

/// The notebook edit, appended **last** by every surface that advertises it.
///
/// It is the one workspace tool added after the tool order was fixed, so it
/// stands after every tool that existed before it. The tool list is a cached
/// prefix: a name inserted in the middle of it discards the cache from that
/// point on for every request of the run, where a name appended at the end
/// leaves the prefix intact. It follows `edit`'s rules and the same write
/// permit; only its place differs.
pub(in crate::agent::tools) fn append_notebook_edit(
    tools: &mut Vec<ToolDefinition>,
    permit_workspace_writes: bool,
) {
    if permit_workspace_writes {
        tools.push(ToolDefinition {
            name: "notebook_edit".into(),
            description: "Change one cell of a Jupyter notebook (`*.ipynb`) in this run's \
                    workspace. `mode` is `replace`, `insert` or `delete`; name the cell by \
                    `index` (0-based) or `cell_id` — exactly one, never `cell_id` for \
                    `insert`. Cells you do not name keep their exact bytes. `edit`'s digest \
                    rule applies. Returns `path`, `mode`, `cell`, `cells`, `size` and \
                    `digest`."
                .into(),
            read_only: false,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Notebook path relative to the workspace root."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["replace", "insert", "delete"],
                        "description": "Which edit to make."
                    },
                    "index": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "The 0-based cell position; for `insert`, the position the new cell takes."
                    },
                    "cell_id": {
                        "type": "string",
                        "description": "The cell's own `id`; `replace` and `delete` only."
                    },
                    "source": {
                        "type": "string",
                        "description": "The cell's new text, for `replace` and `insert`."
                    },
                    "cell_type": {
                        "type": "string",
                        "enum": ["code", "markdown"],
                        "description": "Required for `insert`; for `replace`, changes the cell's type."
                    },
                    "expected_digest": {
                        "type": "string",
                        "description": "The sha256 hex `digest` from a fresh `read`; optional only when this run already read the same file."
                    }
                },
                "required": ["path", "mode"],
                "additionalProperties": false
            }),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::WriteWorkspace,
            },
            concurrency: ToolConcurrency::Serial,
            completion: Some("notebook cell edited".into()),
        });
    }
}

/// Moves the notebook edit to the end of a composed list, for a surface that
/// appends further tools after the workspace block — the coding root's shell,
/// web and plan lanes, the helper's database and web lanes. Every tool that
/// existed before the notebook edit keeps the position it had, so the request
/// carries the same cached prefix it did then. A list with no notebook edit —
/// a read-only surface — is left exactly as it is.
pub(in crate::agent::tools) fn notebook_edit_last(tools: &mut Vec<ToolDefinition>) {
    if let Some(index) = tools.iter().position(|tool| tool.name == "notebook_edit") {
        let definition = tools.remove(index);
        tools.push(definition);
    }
}
