use super::session_state::SessionState;
use crate::slash::{SlashCommand, help_text};
use saya_agent::ApprovalPolicy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionAction {
    Message(String),
    NotImplemented(String),
    Exit,
}

impl SessionState {
    pub fn apply(&mut self, command: SlashCommand, available: &[String]) -> SessionAction {
        match command {
            SlashCommand::Connect(name) => {
                self.profile = Some(name.clone());
                SessionAction::Message(format!("Connected profile: {name}"))
            }
            SlashCommand::Connections => SessionAction::Message(if available.is_empty() {
                "No configured connection profiles.".into()
            } else {
                format!("Profiles: {}", available.join(", "))
            }),
            SlashCommand::Include(name) => {
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
                    self.provider = value;
                }
                SessionAction::NotImplemented(format!(
                    "provider '{}' is configured, but provider execution is not implemented",
                    self.provider
                ))
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
                SessionAction::Message("Conversation context cleared.".into())
            }
            SlashCommand::History => SessionAction::NotImplemented(
                "session history is provided by the store, not the shell state".into(),
            ),
            SlashCommand::Help => SessionAction::Message(help_text().into()),
            SlashCommand::Exit => SessionAction::Exit,
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
