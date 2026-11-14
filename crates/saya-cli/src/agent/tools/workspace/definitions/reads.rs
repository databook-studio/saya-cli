fn append_workspace_read_tools(tools: &mut Vec<ToolDefinition>) {
    // The workspace read is unconditional: it touches no database data, so
    // the privacy gate does not hide it. When no workspace is attached —
    // every path until the run engine passes one in — dispatch denies with
    // a typed error rather than the definition advertising a dead tool
    // silently succeeding. It states its own completion: the generic
    // read-only wording says "database", which a workspace file read is
    // not.
    tools.push(ToolDefinition {
        name: "read".into(),
        description: crate::agent::tools::lean::READ.into(),
        read_only: true,
        parameters: crate::agent::tools::lean::read_parameters(),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::Read,
        },
        concurrency: ToolConcurrency::Concurrent,
        completion: Some("workspace file read".into()),
    });
    // The workspace search tools are unconditional like `read`:
    // they touch no database data, so the privacy gate does not hide
    // them, and each states its own completion — the generic read-only
    // wording says "database", which a workspace search is not.
    tools.push(ToolDefinition {
        name: "list".into(),
        description: "List one directory of this run's workspace — the contained \
                directory holding this run's files. Pass `path` relative to the workspace \
                root (an absolute path under it is accepted); omit it, pass empty, `.`, or \
                `./` to list the root itself. Returns \
                sorted `entries`, each with its `name`, `kind` (`file`, `dir`, \
                `symlink`, or `other`), and `size` in bytes, plus `truncated` when \
                entries were left out and `omitted` counting them — an exact count \
                unless `omitted_exact` is false. Symlinks are reported as entries and \
                never followed, and a directory holding more entries than the bound is \
                served short with the rest counted, never refused. This lists one \
                directory only: for a whole tree in one call use `glob` with a `**/*` \
                pattern."
            .into(),
        read_only: true,
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory path relative to the workspace root; \
                        omit, empty, `.`, or `./` lists the root itself."
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
        concurrency: ToolConcurrency::Concurrent,
        completion: Some("workspace directory listed".into()),
    });
    tools.push(ToolDefinition {
        name: "glob".into(),
        description: crate::agent::tools::lean::GLOB.into(),
        read_only: true,
        parameters: crate::agent::tools::lean::glob_parameters(),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::Read,
        },
        concurrency: ToolConcurrency::Concurrent,
        completion: Some("workspace paths matched".into()),
    });
    tools.push(ToolDefinition {
        name: "grep".into(),
        description: crate::agent::tools::lean::GREP.into(),
        read_only: true,
        parameters: crate::agent::tools::lean::grep_parameters(),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::Read,
        },
        concurrency: ToolConcurrency::Concurrent,
        completion: Some("workspace text searched".into()),
    });
}
