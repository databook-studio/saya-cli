/// The host lane's executor member: the composed config, whose workspace
/// root the child runs with as its cwd, carrying the turn's cancellation.
/// One program per call, typed argv — the runner's own contract reused.
struct HostCommandMember {
    config: saya_harness::host::HostConfig,
    cancellation: CancellationToken,
    /// The session's host-ran flag, set after a host call settles so later
    /// `run_program` prompts gain the staged-binary integrity line.
    host_ran: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl HostCommandMember {
    /// Runs one host call: journals `session-command` before the child
    /// spawns — consent-before-action for the one lane where it matters
    /// most — then resolves, builds, and spawns. A failed journal write
    /// changes no consent: the call still runs, like every other journaling
    /// site's posture (the consent stands, the audit line is missing).
    /// Test shapes without a journal reach the same path with `None`: parse
    /// and spawn with no journal line.
    async fn execute_with_journal(
        &self,
        arguments: serde_json::Value,
        journal: Option<&saya_store::SessionJournal>,
    ) -> Result<serde_json::Value, ToolError> {
        let parsed = self.parse(&arguments)?;
        if let Some(journal) = journal {
            let _ = journal.command(&parsed.program, &parsed.argv, "run_command");
        }
        self.spawn(parsed).await
    }

    fn parse(&self, arguments: &serde_json::Value) -> Result<ParsedHostCall, ToolError> {
        let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
        for key in object.keys() {
            if !matches!(key.as_str(), "program" | "args" | "timeout_seconds") {
                return Err(ToolError::UnsupportedProperty);
            }
        }
        let program = object
            .get("program")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::UnsupportedProperty)?;
        let argv: Vec<String> = match object.get("args") {
            None => Vec::new(),
            Some(serde_json::Value::Array(items)) => {
                // Admission before cloning and before journalling: the bound
                // runs on the borrowed JSON, so an oversized array refuses
                // without materialising an owned copy or a journal line.
                validate_host_json_argv(items)?;
                items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map(str::to_owned)
                            .ok_or(ToolError::UnsupportedProperty)
                    })
                    .collect::<Result<_, _>>()?
            }
            Some(_) => return Err(ToolError::UnsupportedProperty),
        };
        let timeout_seconds = match object.get("timeout_seconds") {
            None => None,
            Some(serde_json::Value::Number(number)) => {
                Some(number.as_u64().ok_or(ToolError::UnsupportedProperty)?)
            }
            Some(_) => return Err(ToolError::UnsupportedProperty),
        };
        Ok(ParsedHostCall {
            program: program.to_owned(),
            argv,
            timeout_seconds,
        })
    }

    async fn spawn(&self, parsed: ParsedHostCall) -> Result<serde_json::Value, ToolError> {
        // The lane has no staging directory; the child's cwd pins to the
        // workspace root through the config — `HostConfig::workspace_root`
        // applied as `current_dir` in `HostCommand::run` — so the prompt's
        // `cwd: pinned to <root>` line is the executor's own fact.
        let command = saya_harness::host::HostCommand::new(parsed.program, parsed.argv)
            .map_err(|error| ToolError::Runner(error.to_string()))?;
        let result = command
            .run(&self.config, parsed.timeout_seconds, &self.cancellation)
            .await
            .map_err(|error| ToolError::Runner(error.to_string()));
        // A settled host call — success or child failure — means a host
        // child ran: later `run_program` prompts gain the integrity line. A
        // refusal (validation, resolution) ran nothing, so the flag stays.
        if result.is_ok()
            && let Some(host_ran) = self.host_ran.as_ref()
        {
            host_ran.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let outcome = result?;
        serde_json::to_value(&outcome)
            .map_err(|_| ToolError::Runner("host outcome failed to render".into()))
    }
}

/// One parsed host call: the bare program, its typed argv, and the call's
/// own timeout narrowing.
struct ParsedHostCall {
    program: String,
    argv: Vec<String>,
    timeout_seconds: Option<u64>,
}
