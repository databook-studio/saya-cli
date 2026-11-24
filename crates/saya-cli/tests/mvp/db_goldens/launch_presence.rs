//! What a golden does with the launch summary's presence.
//!
//! The block is the run's one background fact: the scan starts at the open and
//! the first message never waits for it, so whether a request carries the block
//! is a race (`chat::workspace_summary`), not a behaviour. A golden therefore
//! records the fact and compares everything else — the bytes it measures have
//! the block taken out (`support::launch_summary`), and the count each request
//! recorded is set aside before two records are compared.
//!
//! What is still held to its shape is the rule the block obeys whatever the
//! scan's timing: a request carries it at most once, on the first user turn's
//! message and nowhere else. `support::launch_summary::recorded_count` asserts
//! that where the body can be read; [`assert_rule`] holds a record that was
//! read back from a file to the same rule.

use super::record::{Golden, RequestRecord};

/// The recorded count against the rule, for a golden read back rather than just
/// produced: a request carries the block once or not at all.
pub(crate) fn assert_rule(scenario: &str, requests: &[RequestRecord]) {
    for (index, request) in requests.iter().enumerate() {
        assert!(
            request.launch_summary_count <= 1,
            "{scenario}: request {index} records the launch summary {} times, and it is the run's \
             one launch fact",
            request.launch_summary_count
        );
    }
}

impl Golden {
    /// The same record with every request's launch-summary count set aside,
    /// which is what two records are compared on: the count is the race, and
    /// the recorded bytes already have the block taken out.
    pub(crate) fn without_launch_presence(&self) -> Self {
        let mut cleared = self.clone();
        for request in &mut cleared.requests {
            request.launch_summary_count = 0;
        }
        cleared
    }
}
