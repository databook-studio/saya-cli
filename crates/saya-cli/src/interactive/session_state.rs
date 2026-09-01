use crate::interactive::tui::types::SessionUsage;
use saya_agent::{ChatMessage, ToolMetadata};
use saya_store::{RedactedSession, RedactedToolMetadata, RedactedTurn, SESSION_VERSION};
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
    pub turns: Vec<RedactedTurn>,
    /// In-memory session token accumulator. `#[serde(skip)]` keeps it
    /// out of persisted session files (invariant 2: no new persisted state);
    /// a resumed session starts with a fresh total. `/clear` resets it.
    #[serde(skip)]
    pub(crate) usage: SessionUsage,
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
            turns: Vec::new(),
            usage: SessionUsage::default(),
        }
    }

    pub fn record(&mut self, role: &str, content: impl Into<String>) {
        self.messages.push(SessionLine {
            role: role.into(),
            content: content.into(),
        });
    }

    pub fn record_turn(
        &mut self,
        user: impl Into<String>,
        assistant: impl Into<String>,
        database_derived: bool,
        tools: Vec<ToolMetadata>,
    ) {
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
                })
                .collect(),
        });
    }

    pub fn provider_history(&self) -> Vec<ChatMessage> {
        let include_sensitive =
            self.provider.eq_ignore_ascii_case("ollama") || self.allow_data_sharing;
        self.turns
            .iter()
            .filter(|turn| include_sensitive || !turn.database_derived)
            .flat_map(|turn| {
                [
                    ChatMessage::text("user", turn.user.clone()),
                    ChatMessage::text("assistant", turn.assistant.clone()),
                ]
            })
            .collect()
    }

    pub fn redacted(&self) -> RedactedSession {
        RedactedSession {
            version: SESSION_VERSION,
            id: self.id.clone(),
            profile: self.profile.clone(),
            included_profiles: self.included_profiles.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            allow_data_sharing: self.allow_data_sharing,
            approval_mode: self.approval_mode.clone(),
            turns: self.turns.clone(),
            profile_names: self.profile_names(),
            messages: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// the CLI-boundary slice deliverable 4 / invariant 2 (non-persistence survives the crossing).
    /// A turn that carried chain-of-thought — surfaced this slice as
    /// `AgentEvent::ReasoningText` — must leave none of it in a persisted session.
    /// `SessionLine` and `RedactedTurn` carry `role` + `content` only; the
    /// reasoning lives on `ChatResponse.reasoning` (transport for one call) and
    /// `AgentEvent::ReasoningText` (an in-memory event), neither of which has a
    /// field on the persisted types. So `record_turn` — the only write path into
    /// a session — has nowhere to copy the reasoning, however hard a caller
    /// tries. This pins that: serialize a session built from a reasoning turn
    /// and assert neither the persisted JSON nor the replayed provider history
    /// contains the reasoning text. If a reasoning field is ever added to
    /// `SessionLine` or `RedactedTurn`, this test fails and the reviewer must
    /// justify letting reasoning reach a persisted session.
    #[test]
    fn a_session_persisted_after_a_reasoning_turn_contains_none_of_it() {
        let reasoning = "the secret chain-of-thought about row values 9f3a";
        // The turn's answer, as the loop assembles it. The reasoning the turn
        // produced is not an argument to `record_turn` and never could be — the
        // signature takes `user`, `assistant`, `database_derived`, `tools`.
        let mut session =
            SessionState::new("s1", Some(String::from("analytics")), String::from("m"));
        session.record_turn("what is the answer", "the answer is 42", false, Vec::new());

        // The serialized form a session file would write.
        let json = serde_json::to_string(&session).expect("serializes");
        assert!(
            !json.contains(reasoning),
            "reasoning leaked into the persisted session: {json}"
        );
        // No `reasoning` key exists on `SessionLine` or `RedactedTurn`; a
        // fabricated one must not appear.
        assert!(
            !json.contains("reasoning"),
            "a `reasoning` key appeared in the persisted session: {json}"
        );

        // And the replay path: `provider_history` rebuilds the messages sent
        // back to the model on a later turn. Reasoning must not be replayed
        // — it cannot be, because the history is built from
        // `SessionLine`/`RedactedTurn` content, which carries only the answer.
        let replayed = session.provider_history();
        assert!(
            replayed.iter().all(|m| !m.content.contains(reasoning)),
            "reasoning leaked into replayed provider history: {replayed:?}"
        );
        // The redacted form (what a session file actually stores) is the same.
        let redacted = session.redacted();
        let redacted_json = serde_json::to_string(&redacted).expect("serializes");
        assert!(
            !redacted_json.contains(reasoning),
            "reasoning leaked into the redacted session: {redacted_json}"
        );
        assert!(
            !redacted_json.contains("reasoning"),
            "a `reasoning` key appeared in the redacted session: {redacted_json}"
        );
    }
}
