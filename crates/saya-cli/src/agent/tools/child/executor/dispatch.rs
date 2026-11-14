//! The child executor's dispatch doors: admission, then the anchored
//! mutation. Split from the composition beside it — the same surface's
//! attachment and admission API — so each half stays one concern.

use super::*;

/// Admission happens immediately before the existing anchored mutation; the
/// checkout id never comes from model arguments; memory tools are
/// absent: unknown names are unsupported. `shell` routes to the helper's own
/// shell when the composition root attached one, and is unknown otherwise; the
/// standard read-only database tools route to the job's profile-admitted
/// toolset, and are unknown when no profile was selected. `run_command` is
/// never routed.
#[async_trait]
impl ToolExecutor for ChildTools {
    fn turn_context_note(&self) -> Option<String> {
        let block = crate::interactive::session::tasks::render::render_tasks_block(
            &self.tasks.take_changed()?,
        )?;
        Some(saya_agent::render_untrusted_block(&block))
    }

    /// The helper's list is re-read before each request, like the root's, so a
    /// background start made in one turn puts `shell_output` and `shell_kill`
    /// in the next turn's tool list. The list only grows once for this cause.
    fn advertised_definitions(&self) -> Option<Vec<ToolDefinition>> {
        Some(self.advertisement())
    }

    /// A helper's background runs belong to the helper's own shell, so its
    /// loop sees their exits the same way the root's does.
    fn take_background_exits(&self) -> Vec<saya_agent::BackgroundExit> {
        self.shell
            .as_ref()
            .map(|shell| shell.take_background_exits())
            .unwrap_or_default()
    }

    fn background_live(&self) -> bool {
        self.shell
            .as_ref()
            .is_some_and(|shell| shell.background_live())
    }

    fn background_notice_pending(&self) -> bool {
        self.shell
            .as_ref()
            .is_some_and(|shell| shell.background_notice_pending())
    }

    async fn wait_background_exit(
        &self,
        deadline: std::time::Instant,
        cancellation: &saya_agent::CancellationToken,
    ) -> bool {
        match &self.shell {
            Some(shell) => shell.wait_background_exit(deadline, cancellation).await,
            None => false,
        }
    }

    async fn execute_with_context(
        &self,
        name: &str,
        arguments: serde_json::Value,
        context: saya_agent::ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        let name = workspace::canonical_name(name);
        // The job's `[tools] allow` list, minus `helpers`, and this helper's
        // own notebook answer, first: a name either removed is unknown here,
        // the same refusal a member this surface never composed gets, so the
        // advertisement and the dispatch cannot drift.
        if !self.admits(name) {
            return Err(ToolError::UnsupportedTool);
        }
        // The helper's shell is the root's tool, so its lane decision — host
        // versus sandbox, and who is asked — is the shell's own. This executor
        // only routes the call; with no shell attached the name is unknown.
        // A start that succeeds is what makes the two readers reachable from
        // the next request on.
        if shell_policy::operation(name).is_some() {
            let Some(shell) = &self.shell else {
                return Err(ToolError::UnsupportedTool);
            };
            let started = shell_policy::starts_background(name, &arguments);
            let outcome = shell.call(name, &arguments).await;
            if started && outcome.is_ok() {
                self.background_started.store(true, Ordering::Release);
            }
            return outcome;
        }
        // The helper's web tools are the root's too, so their consent rule —
        // the mode, and who is asked — is each tool's own. This executor only
        // routes the call; with no lane attached, or with a name the lane
        // holds no tool for, the name is unknown.
        if crate::agent::tools::web::is_web_tool(name) {
            let Some(web) = &self.web else {
                return Err(ToolError::UnsupportedTool);
            };
            return web.call(name, &arguments).await;
        }
        // The helper's MCP tools are the root's too, so each call's consent —
        // the mode, and who is asked — is the lane's own. This executor only
        // routes the name; with no lane attached it is unknown.
        if crate::mcp::client::McpTools::owns(name) {
            let Some(mcp) = &self.mcp else {
                return Err(ToolError::UnsupportedTool);
            };
            return mcp.execute_with_context(name, arguments, context).await;
        }
        if matches!(name, "grep" | "glob") {
            workspace::validate_search(name, &arguments, &context)?;
            let private_prefixes = if let Some(coding) = &self.coding {
                coding
                    .search_private_prefixes(&context.cancellation)
                    .await?
            } else {
                Vec::new()
            };
            return workspace::execute_search(
                &self.tools,
                name,
                &arguments,
                context,
                private_prefixes,
            )
            .await;
        }
        if name == "delete" {
            self.admit_mutation(&arguments)?;
            let coding = self
                .coding
                .as_ref()
                .ok_or_else(|| ToolError::NoEffect(Box::new(ToolError::UnsupportedTool)))?;
            return workspace::delete_owned(coding, arguments, context).await;
        }
        if crate::commands::coding::database::routes_here(name) {
            let databases = self.databases.as_ref().ok_or(ToolError::UnsupportedTool)?;
            return databases
                .execute(Some(&self.view), name, arguments, context)
                .await;
        }
        if workspace::is_tool_name(name) {
            // The notebook edit is admitted exactly as `edit` is: it is the
            // same write, over one cell's bytes.
            let mutation = matches!(name, "write" | "edit" | "notebook_edit");
            if mutation {
                self.admit_mutation(&arguments)?;
            }
            let cancellation = context.cancellation.clone();
            let result_cap = context.result_cap;
            return match workspace::execute_with_context(&self.tools, name, &arguments, context)
                .await
            {
                Ok(result) if matches!(name, "write" | "edit" | "notebook_edit") => {
                    self.after_workspace_mutation(name, result, &cancellation, result_cap)
                        .await
                }
                Err(error) if mutation && super::super::no_effect::proves_no_effect(&error) => {
                    Err(ToolError::NoEffect(Box::new(error)))
                }
                outcome => outcome,
            };
        }
        self.execute(name, arguments).await
    }
    async fn execute_with_result_cap(
        &self,
        name: &str,
        arguments: serde_json::Value,
        result_cap: usize,
    ) -> Result<serde_json::Value, ToolError> {
        if workspace::is_tool_name(name) {
            let cancellation = self
                .coding
                .as_ref()
                .map(|owner| owner.ledger.cancellation_token())
                .unwrap_or_default();
            self.execute_with_context(
                name,
                arguments,
                saya_agent::ToolExecutionContext {
                    result_cap,
                    cancellation,
                },
            )
            .await
        } else {
            self.execute(name, arguments).await
        }
    }

    async fn settle_invocation(&self) -> Result<(), ToolError> {
        self.tools.settle_invocation().await?;
        Ok(())
    }

    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        // `bash` is the model's spelling of `shell`; every other name is taken
        // as advertised.
        let name = workspace::canonical_name(name);
        // The same gate `execute_with_context` applies, so this door — and
        // `execute_with_result_cap`, which routes every non-workspace name
        // here — cannot serve a name the advertisement dropped.
        if !self.admits(name) {
            return Err(ToolError::UnsupportedTool);
        }
        // The same route the context door takes: an MCP name is the lane's,
        // and is unknown where no lane was attached.
        if crate::mcp::client::McpTools::owns(name) {
            let Some(mcp) = &self.mcp else {
                return Err(ToolError::UnsupportedTool);
            };
            return mcp.execute(name, arguments).await;
        }
        match name {
            "checklist" => self.tasks.execute(name, arguments).await,
            // The ask needs no context, so this door serves it directly,
            // through the same advertised gate the context door applies.
            name if crate::agent::tools::clarification::is_ask(name) => {
                let databases = self.databases.as_ref().ok_or(ToolError::UnsupportedTool)?;
                databases.ask_result().ok_or(ToolError::UnsupportedTool)
            }
            name if shell_policy::operation(name).is_some() => {
                let cancellation = self
                    .coding
                    .as_ref()
                    .map(|owner| owner.ledger.cancellation_token())
                    .unwrap_or_default();
                return self
                    .execute_with_context(
                        name,
                        arguments,
                        saya_agent::ToolExecutionContext {
                            result_cap: 64 * 1024,
                            cancellation,
                        },
                    )
                    .await;
            }
            "delete" | "read" | "list" | "write" | "edit" | "notebook_edit" | "glob" | "grep"
            | "web_fetch" | "web_search" | "schema_discovery" | "bounded_sql_query" => {
                let cancellation = self
                    .coding
                    .as_ref()
                    .map(|owner| owner.ledger.cancellation_token())
                    .unwrap_or_default();
                return self
                    .execute_with_context(
                        name,
                        arguments,
                        saya_agent::ToolExecutionContext {
                            result_cap: 64 * 1024,
                            cancellation,
                        },
                    )
                    .await;
            }
            _ => return Err(ToolError::UnsupportedTool),
        }
    }
}
