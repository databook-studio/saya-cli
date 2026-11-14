//! Manual `/compact`: plan, validate, apply, and the summarising call's
//! failure posture. A failed compaction never damages the session — every
//! failure path below leaves the conversation exactly as it was.

use crate::interactive::session::continuation::compaction::manual::{
    apply, failure_message, pinned_tokens, plan, success_message, validate,
};
use crate::interactive::session::continuation::compaction::provider::summarise;
use crate::interactive::session::state::SessionState;
use async_trait::async_trait;
use saya_agent::{ChatMessage, ChatProvider, ChatRequest, ChatResponse, ProviderError};

fn turns(count: usize) -> SessionState {
    let mut state = SessionState::new("s1", None, "m");
    for index in 0..count {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    state
}

struct StaticProvider {
    text: String,
    usage: Option<saya_agent::TokenUsage>,
}

#[async_trait]
impl ChatProvider for StaticProvider {
    fn name(&self) -> &str {
        "static"
    }
    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let mut response = ChatResponse::new(ChatMessage::text("assistant", &self.text));
        response.usage = self.usage;
        Ok(response)
    }
}

struct ErrorProvider;

#[async_trait]
impl ChatProvider for ErrorProvider {
    fn name(&self) -> &str {
        "error"
    }
    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::Request("http 500 server error".into()))
    }
}

struct TruncatedProvider;

#[async_trait]
impl ChatProvider for TruncatedProvider {
    fn name(&self) -> &str {
        "truncated"
    }
    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::output_truncated(
            "partial".into(),
            Vec::new(),
        ))
    }
}

include!("manual_tests/planning.rs");

include!("manual_tests/provider_failures.rs");

include!("manual_tests/no_op_usage.rs");

include!("manual_tests/surface.rs");
