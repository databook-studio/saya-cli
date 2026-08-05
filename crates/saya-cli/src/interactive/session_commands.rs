use super::session_state::SessionState;
use crate::agent::runtime::PromptOverrides;
use crate::slash::SlashCommand;
use saya_agent::AgentOutput;
use saya_agent::ApprovalPolicy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionAction {
    Message(String),
    Agent(AgentOutput),
    Cancelled,
    NotImplemented(String),
    Error(String),
    History,
    Resume(String),
    Schema(bool),
    Sql(String),
    Exit,
}

impl SessionState {
    pub fn apply(&mut self, command: SlashCommand, available: &[String]) -> SessionAction {
        match command {
            SlashCommand::Connect(name) => {
                if !available.iter().any(|profile| profile == &name) {
                    return SessionAction::Error(format!("Unknown configured profile: {name}"));
                }
                self.profile = Some(name.clone());
                SessionAction::Message(format!("Selected profile: {name}"))
            }
            SlashCommand::Connections => SessionAction::Message(if available.is_empty() {
                "No configured connection profiles.".into()
            } else {
                format!("Profiles: {}", available.join(", "))
            }),
            SlashCommand::Include(name) => {
                if !available.iter().any(|profile| profile == &name) {
                    return SessionAction::Error(format!("Unknown configured profile: {name}"));
                }
                if !self.included_profiles.contains(&name) {
                    self.included_profiles.push(name.clone());
                }
                SessionAction::Message(format!("Included profile: {name}"))
            }
            SlashCommand::Exclude(name) => {
                self.included_profiles.retain(|item| item != &name);
                SessionAction::Message(format!("Excluded profile: {name}"))
            }
            SlashCommand::Provider(value) => {
                if let Some(value) = value {
                    if saya_config::AiProvider::parse(&value).is_none() {
                        return SessionAction::Error(format!(
                            "Unsupported provider: {value}. Use ollama, openai, openai_compatible, anthropic, or gemini."
                        ));
                    }
                    self.provider = value;
                    SessionAction::Message(format!("Provider: {}", self.provider))
                } else {
                    SessionAction::Message(format!(
                        "Provider: {} (available: {})",
                        self.provider,
                        available_providers().join(", ")
                    ))
                }
            }
            SlashCommand::Model(value) => {
                if let Some(value) = value {
                    self.model = value;
                    SessionAction::Message(format!("Model: {}", self.model))
                } else {
                    let models = known_models(&self.provider);
                    if models.is_empty() {
                        SessionAction::Message(format!(
                            "Model: {} (no suggestions for provider {})",
                            self.model, self.provider
                        ))
                    } else {
                        SessionAction::Message(format!(
                            "Model: {}\nKnown models for {}: {}",
                            self.model,
                            self.provider,
                            models.join(", ")
                        ))
                    }
                }
            }
            SlashCommand::Privacy(value) => {
                if let Some(value) = value {
                    self.allow_data_sharing = value;
                }
                SessionAction::Message(format!(
                    "Cloud data sharing: {}",
                    if self.allow_data_sharing {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ))
            }
            SlashCommand::Approvals(value) => {
                if let Some(value) = value {
                    self.approval_mode = approval_name(value);
                }
                SessionAction::Message(format!("Approval mode: {}", self.approval_mode))
            }
            SlashCommand::Schema(refresh) => SessionAction::Schema(refresh),
            SlashCommand::Sql(query) => SessionAction::Sql(query),
            SlashCommand::Clear => {
                self.messages.clear();
                self.turns.clear();
                SessionAction::Message("Conversation context cleared.".into())
            }
            SlashCommand::History => SessionAction::History,
            SlashCommand::Sessions => SessionAction::History,
            SlashCommand::Resume(id) => SessionAction::Resume(id),
            SlashCommand::Help(topic) => {
                SessionAction::Message(crate::slash::help_for(topic.as_deref()))
            }
            SlashCommand::Exit => SessionAction::Exit,
        }
    }

    pub(crate) fn prompt_overrides(&self) -> PromptOverrides {
        PromptOverrides {
            provider: saya_config::AiProvider::parse(&self.provider),
            model: Some(self.model.clone()),
            allow_data_sharing: Some(self.allow_data_sharing),
            profile: self.profile.clone(),
            included_profiles: self.included_profiles.clone(),
        }
    }
}

fn approval_name(policy: ApprovalPolicy) -> String {
    match policy {
        ApprovalPolicy::Ask => "ask",
        ApprovalPolicy::ReadOnly => "read-only",
        ApprovalPolicy::Never => "never",
    }
    .into()
}

fn available_providers() -> &'static [&'static str] {
    &[
        "ollama",
        "openai",
        "openai_compatible",
        "anthropic",
        "gemini",
    ]
}

/// Curated suggestions for models per provider (convenience suggestions, not an exhaustive or validated list).
fn known_models(provider: &str) -> &'static [&'static str] {
    match provider.to_ascii_lowercase().as_str() {
        "ollama" => &["qwen2.5-coder:14b", "llama3.1", "mistral"],
        "openai" => &["gpt-4o", "gpt-4o-mini", "o3-mini"],
        "anthropic" => &["claude-sonnet-4", "claude-opus-4", "claude-3-5-haiku"],
        "gemini" => &["gemini-2.0-flash", "gemini-1.5-pro"],
        "openai_compatible" => &[],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_none_lists_available() {
        let mut state = SessionState::new("test", None, "gpt-4o");
        let action = state.apply(SlashCommand::Provider(None), &[]);
        if let SessionAction::Message(msg) = action {
            assert!(msg.contains("available:"));
            assert!(msg.contains("anthropic"));
        } else {
            panic!("Expected SessionAction::Message");
        }
    }

    #[test]
    fn test_model_none_lists_known_models_for_ollama() {
        let mut state = SessionState::new("test", None, "qwen2.5-coder:14b");
        state.provider = "ollama".into();
        let action = state.apply(SlashCommand::Model(None), &[]);
        if let SessionAction::Message(msg) = action {
            assert!(msg.contains("qwen2.5-coder:14b"));
        } else {
            panic!("Expected SessionAction::Message");
        }
    }

    #[test]
    fn test_provider_some_sets_provider_without_available() {
        let mut state = SessionState::new("test", None, "gpt-4o");
        let action = state.apply(SlashCommand::Provider(Some("openai".into())), &[]);
        assert_eq!(state.provider, "openai");
        if let SessionAction::Message(msg) = action {
            assert_eq!(msg, "Provider: openai");
            assert!(!msg.contains("available:"));
        } else {
            panic!("Expected SessionAction::Message");
        }
    }
}
