//! The turn's approval decider: the modal over a channel.

mod decider;
mod sink;
mod turn;

pub(crate) use decider::ChannelApproval;
pub(crate) use sink::ChannelSink;
pub(crate) use turn::{StreamRequest, approval_capabilities};
