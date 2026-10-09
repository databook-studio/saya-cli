#[async_trait]
impl ToolExecutor for RunTools {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.execute_with_context(
            name,
            arguments,
            ToolExecutionContext {
                result_cap: self.database.context_byte_budget,
                cancellation: CancellationToken::new(),
            },
        )
        .await
    }

    async fn execute_with_result_cap(
        &self,
        name: &str,
        arguments: serde_json::Value,
        result_cap: usize,
    ) -> Result<serde_json::Value, ToolError> {
        self.execute_with_context(
            name,
            arguments,
            ToolExecutionContext {
                result_cap,
                cancellation: CancellationToken::new(),
            },
        )
        .await
    }

    async fn execute_with_context(
        &self,
        name: &str,
        arguments: serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        if name == "render_chart" && arguments.get("save_to").is_some() && !self.chart_save_permit {
            return Err(ToolError::WorkspaceWrite(
                "workspace-write is not permitted for this step".into(),
            ));
        }
        // Deny first, at every program-named door: before grant lookup,
        // before the approval prompt, before bypass's auto-allow. Deny is a
        // structural refusal — it holds in every mode, bypass included.
        if matches!(name, "run_command" | "run_program") {
            let door = crate::interactive::session::permissions::deny::call_door(name);
            if let Some(program) =
                crate::interactive::session::permissions::deny::call_program(name, &arguments)
                && self.deny.contains(&program)
            {
                // The deny journal line must not materialise unbounded argv
                // either: validate the raw JSON first, then carry at most the
                // admitted bound into the owned strings the journal redacts.
                if name == "run_command"
                    && let Some(serde_json::Value::Array(items)) = arguments.get("args")
                {
                    validate_host_json_argv(items)?;
                }
                let argv: Vec<String> = arguments
                    .get("args")
                    .and_then(serde_json::Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(journal) = self.journal.as_ref() {
                    let _ = journal.command_denied(&program, &argv, door);
                }
                return Err(ToolError::Runner(
                    crate::interactive::session::permissions::deny::denied_refusal(&program),
                ));
            }
        }
        match name {
            "scratch_sql" | "scratch_import" => match &self.scratch {
                Some(scratch) => scratch.execute_with_context(name, arguments, context).await,
                None => Err(ToolError::UnsupportedTool),
            },
            "http_fetch" | "http_download" => match &self.fetch {
                Some(fetch) => fetch.execute_with_context(name, arguments, context).await,
                None => Err(ToolError::UnsupportedTool),
            },
            "run_program" => match &self.runner {
                Some(runner) => runner.execute(name, arguments).await,
                None => Err(ToolError::UnsupportedTool),
            },
            "run_command" => match &self.host {
                Some(host) => {
                    host.execute_with_journal(arguments, self.journal.as_deref())
                        .await
                }
                None => Err(ToolError::UnsupportedTool),
            },
            // The session task list: whole-list replace against the shared
            // cell. Absent outside a session — a run has no session list —
            // where the name refuses as unknown, the hidden-not-advertised
            // discipline at dispatch.
            "tasks_set" => match &self.tasks {
                Some(tasks) => {
                    crate::interactive::session::tasks::TasksSet::new(tasks.clone())
                        .execute(name, arguments)
                        .await
                }
                None => Err(ToolError::UnsupportedTool),
            },
            _ => {
                self.database
                    .execute_with_context(name, arguments, context)
                    .await
            }
        }
    }
}
