//! Chat conversation contracts: the agent request, the messages replayed to a
//! provider, and the provider call's request/response and intent options.

mod call;
mod intent;
mod message;
mod session;
mod tool_result;

pub use call::{ChatRequest, ChatResponse};
pub use intent::{ReasoningEffort, ResponseFormat};
pub use message::{ChatMessage, ToolCall};
pub use session::{AgentRequest, ContextBlock};
pub use tool_result::{ToolMetadata, ToolResultShape};
