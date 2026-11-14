//! Chat conversation contracts: the agent request, the messages replayed to a
//! provider, and the provider call's request/response and intent options.

mod call;
mod image;
mod intent;
mod message;
mod session;
mod tool_result;

pub use call::{ChatRequest, ChatResponse};
pub use image::{IMAGE_RESULT_FIELD, ImageBlock, take_image_blocks};
pub use intent::{ReasoningEffort, ResponseFormat};
pub use message::{ChatMessage, MAX_REPLAYED_REASONING_BYTES, ToolCall};
pub use session::{AgentRequest, ContextBlock};
pub use tool_result::{ToolMetadata, ToolResultShape};
