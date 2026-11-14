#[path = "history/assembly.rs"]
mod assembly;

#[cfg(test)]
use crate::{AgentError, ChatMessage, ContextBlock};
#[cfg(test)]
use assembly::{SYSTEM_PROMPT, message_bytes};
pub use assembly::{build_messages, turn_bytes};
pub(crate) use assembly::{build_messages_with_history_count, discard_oldest_history_pair};

#[cfg(test)]
#[path = "history/tests.rs"]
mod tests;
