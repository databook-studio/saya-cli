impl SessionState {
    pub fn redacted(&self) -> RedactedSession {
        RedactedSession {
            version: SESSION_VERSION,
            id: self.id.clone(),
            profile: self.profile.clone(),
            included_profiles: self.included_profiles.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            provider_endpoint: self.provider_endpoint.clone(),
            provider_endpoint_bound: self.provider_endpoint_bound,
            allow_data_sharing: self.allow_data_sharing,
            approval_mode: self.approval_mode.clone(),
            agent_mode: self.agent_mode.clone(),
            workspace_root: self.workspace_root.clone(),
            task_list: self.task_list.clone(),
            turns: self.turns.clone(),
            profile_names: self.profile_names(),
            messages: Vec::new(),
            continuation: self.continuation.clone(),
            parent: self.parent.clone(),
        }
    }

    fn profile_names(&self) -> Vec<String> {
        self.profile
            .iter()
            .cloned()
            .chain(self.included_profiles.iter().cloned())
            .collect()
    }

    /// The session's task posture, parsed from the stored spelling. An
    /// unknown or legacy-absent value falls back to Build — the composition
    /// root reads this, never the raw string, so a corrupt or pre-`/mode`
    /// session still turns exactly as before.
    pub fn agent_mode_parsed(&self) -> saya_agent::AgentMode {
        self.agent_mode.parse().unwrap_or_default()
    }

    /// The session's consent posture, parsed from the stored spelling. The
    /// stored spelling is the *session's* own vocabulary, which is wider than
    /// the launch grammar's by exactly one word: `accept-edits`, which only
    /// the session's permission-mode switch can set. [`ApprovalPolicy`]'s own
    /// parse deliberately refuses it — that refusal is what keeps a flag, a
    /// config file and a resumed record from reaching the posture — so an
    /// in-session reader comes through here, and a record reader does not.
    /// Without this the switch would be undone at the next turn, whose start
    /// re-parses the stored word.
    ///
    /// The refusal is preserved for every other word: a reader that states a
    /// mode to the engine propagates it (a resumed record carrying a spelling
    /// no binary knows fails the run rather than turning under a posture the
    /// record never named), and a reader that only displays one falls back to
    /// `Ask`, the closed direction.
    ///
    /// [`ApprovalPolicy`]: saya_agent::ApprovalPolicy
    pub fn approval_mode_parsed(
        &self,
    ) -> Result<saya_agent::ApprovalPolicy, saya_agent::ApprovalPolicyParseError> {
        match self.approval_mode.as_str() {
            "accept-edits" => Ok(saya_agent::ApprovalPolicy::AcceptEdits),
            other => other.parse(),
        }
    }
}
