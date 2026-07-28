use super::session_state::SessionState;
use crate::agent_runtime::PromptOverrides;
use crate::slash::{SlashCommand, help_text};
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
                SessionAction::Message(format!("Included profile (display-only): {name}"))
            }
            SlashCommand::Exclude(name) => {
                self.included_profiles.retain(|item| item != &name);
                SessionAction::Message(format!("Excluded profile: {name}"))
            }
            SlashCommand::Provider(value) => {
                if let Some(value) = value {
                    let supported = matches!(
                        saya_config::AiProvider::parse(&value),
                        Some(
                            saya_config::AiProvider::Ollama
                                | saya_config::AiProvider::Openai
                                | saya_config::AiProvider::OpenaiCompatible
                        )
                    );
                    if !supported {
                        return SessionAction::Error(format!(
                            "Unsupported provider: {value}. Use ollama, openai, or openai_compatible."
                        ));
                    }
                    self.provider = value;
                }
                SessionAction::Message(format!("Provider: {}", self.provider))
            }
            SlashCommand::Model(value) => {
                if let Some(value) = value {
                    self.model = value;
                }
                SessionAction::Message(format!("Model: {}", self.model))
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
            SlashCommand::Schema(refresh) => SessionAction::NotImplemented(if refresh {
                "schema refresh is not implemented".into()
            } else {
                "schema inspection is not implemented".into()
            }),
            SlashCommand::Clear => {
                self.messages.clear();
                self.turns.clear();
                SessionAction::Message("Conversation context cleared.".into())
            }
            SlashCommand::History => SessionAction::History,
            SlashCommand::Help => SessionAction::Message(help_text().into()),
            SlashCommand::Exit => SessionAction::Exit,
        }
    }

    pub(crate) fn prompt_overrides(&self) -> PromptOverrides {
        PromptOverrides {
            provider: saya_config::AiProvider::parse(&self.provider),
            model: Some(self.model.clone()),
            allow_data_sharing: Some(self.allow_data_sharing),
            profile: self.profile.clone(),
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
