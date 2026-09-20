//! Runs an agent prompt on a background thread and streams its events back to
//! the UI over a channel, so the event loop stays responsive (spinner + cancel)
//! while the model works.

pub(crate) mod approval;
pub(crate) mod messages;
pub(crate) mod spawn;

#[allow(unused_imports)]
pub(crate) use approval::{ChannelApproval, StreamRequest, approval_capabilities};
pub(crate) use messages::{Stream, StreamMsg};
pub(crate) use spawn::start;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
