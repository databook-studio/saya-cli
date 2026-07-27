use saya_store::{RedactedMessage, RedactedSession};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionState {
    pub id: String,
    pub profile: Option<String>,
    pub included_profiles: Vec<String>,
    pub provider: String,
    pub model: String,
    pub allow_data_sharing: bool,
    pub approval_mode: String,
    pub messages: Vec<SessionLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionLine {
    pub role: String,
    pub content: String,
}
pub type Session = SessionState;

impl SessionState {
    pub fn new(id: impl Into<String>, profile: Option<String>, model: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            profile,
            included_profiles: Vec::new(),
            provider: "ollama".into(),
            model: model.into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
            messages: Vec::new(),
        }
    }

    pub fn record(&mut self, role: &str, content: impl Into<String>) {
        self.messages.push(SessionLine {
            role: role.into(),
            content: content.into(),
        });
    }

    pub fn redacted(&self) -> RedactedSession {
        RedactedSession {
            id: self.id.clone(),
            profile_names: self.profile_names(),
            messages: self
                .messages
                .iter()
                .map(|line| RedactedMessage {
                    role: line.role.clone(),
                    content: line.content.clone(),
                })
                .collect(),
        }
    }

    fn profile_names(&self) -> Vec<String> {
        self.profile
            .iter()
            .cloned()
            .chain(self.included_profiles.iter().cloned())
            .collect()
    }
}
