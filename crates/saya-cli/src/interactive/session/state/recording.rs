impl SessionState {
    pub fn new(id: impl Into<String>, profile: Option<String>, model: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            profile,
            included_profiles: Vec::new(),
            provider: "ollama".into(),
            model: model.into(),
            provider_endpoint: None,
            provider_endpoint_bound: false,
            allow_data_sharing: false,
            approval_mode: "ask".into(),
            agent_mode: default_agent_mode(),
            workspace_root: None,
            task_list: saya_types::SessionTaskList::default(),
            prior_tool_outcomes: None,
            observations: crate::interactive::session::continuation::SessionObservations::default(),
            continuation: None,
            parent: None,
            messages: Vec::new(),
            turns: Vec::new(),
            compaction_summary: None,
            compacted_turns: 0,
            show_thinking: false,
            usage: SessionUsage::default(),
            host_composed: false,
            context_warned: default_context_warned(),
            capacity_warned: default_capacity_warned(),
            auto_compact_failed: default_auto_compact_failed(),
            denied_programs: Vec::new(),
        }
    }

    pub fn record(&mut self, role: &str, content: impl Into<String>) {
        self.messages.push(SessionLine {
            role: role.into(),
            content: content.into(),
        });
    }

    /// Binds a legacy or fresh session to the current runtime endpoint once.
    /// A persisted explicit clear is left untouched so switching provider
    /// cannot silently restore the old endpoint on the next turn.
    pub(crate) fn bind_runtime_endpoint(&mut self, endpoint: Option<&str>) {
        if !self.provider_endpoint_bound {
            self.provider_endpoint = endpoint.map(str::to_owned);
            self.provider_endpoint_bound = true;
        }
    }

    /// Re-derives the runtime-only host-posture facts (the status header's
    /// `host:` segment and its deny list) from the session's live universe.
    /// A state swapped in after a `reacquire` — a `/resume` or a `/rotate` —
    /// arrives with both reset (the `#[serde(skip)]` defaults), while the
    /// recomposed universe composes the host lane wherever a workspace root
    /// binds: the header must say what the swapped-in session will actually
    /// run, never what the previous state carried.
    pub(crate) fn sync_host_posture(&mut self, universe: &SessionUniverse) {
        self.host_composed = universe.host_composed();
        self.denied_programs = universe.deny_programs();
    }

    pub fn record_turn(
        &mut self,
        user: impl Into<String>,
        assistant: impl Into<String>,
        database_derived: bool,
        tools: Vec<ToolMetadata>,
    ) {
        self.prior_tool_outcomes = None;
        let user = user.into();
        let assistant = assistant.into();
        self.messages.push(SessionLine {
            role: "user".into(),
            content: user.clone(),
        });
        self.messages.push(SessionLine {
            role: "assistant".into(),
            content: assistant.clone(),
        });
        self.turns.push(RedactedTurn {
            user,
            assistant,
            database_derived,
            tools: tools
                .into_iter()
                .map(|tool| RedactedToolMetadata {
                    name: tool.name,
                    status: tool.status,
                    arguments: tool.arguments,
                    result_shape: tool.result_shape.map(|shape| RedactedToolResultShape {
                        row_count: shape.row_count,
                        columns: shape.columns,
                    }),
                })
                .collect(),
        });
    }

    /// Records a completed turn and its bounded live outcome observation.
    /// Status strings are interpreted only here, while they belong to the
    /// actual runtime output; resumed strings never feed this projection.
    pub(crate) fn record_agent_output(&mut self, user: impl Into<String>, output: &AgentOutput) {
        self.record_turn(
            user,
            output.answer.clone(),
            output.used_bounded_sql_query,
            output.tool_metadata.clone(),
        );
        self.prior_tool_outcomes = Some(
            crate::interactive::session::continuation::PriorToolOutcomes::from_live_output(output),
        );
        self.observations.absorb(
            output,
            crate::interactive::session::continuation::now_unix_ms(),
        );
    }

    pub fn provider_history(&self) -> Vec<ChatMessage> {
        let mut history = self.provider_request_history();
        if let Some(summary) = self.compaction_narrative() {
            history.insert(0, ChatMessage::text("user", summary));
        }
        history
    }

    pub(crate) fn provider_request_history(&self) -> Vec<ChatMessage> {
        let provider = saya_config::AiProvider::parse(&self.provider)
            .unwrap_or(saya_config::AiProvider::Ollama);
        let include_sensitive = crate::agent::runtime::query_data_allowed_for_endpoint(
            provider,
            self.provider_endpoint.as_deref(),
            self.allow_data_sharing,
        );
        let mut history: Vec<ChatMessage> = self
            .turns
            .iter()
            .filter(|turn| include_sensitive || !turn.database_derived)
            .flat_map(|turn| {
                [
                    ChatMessage::text("user", turn.user.clone()),
                    ChatMessage::text("assistant", turn.assistant.clone()),
                ]
            })
            .collect();
        if self.compaction_narrative().is_some() {
            let drop = (self.compacted_turns * 2).min(history.len());
            history.drain(..drop);
        }
        history
    }
}
