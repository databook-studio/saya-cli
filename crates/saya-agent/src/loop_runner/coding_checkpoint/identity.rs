//! Private comparability references; bounded by validated owner fields.
use super::Fact;

impl Fact {
    pub(super) fn same_key(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::File { path: a, .. } | Self::Deleted { path: a, .. },
                Self::File { path: b, .. } | Self::Deleted { path: b, .. },
            ) => a == b,
            (
                Self::FollowUp {
                    agent_id: a,
                    invocation_id: x,
                    ..
                },
                Self::FollowUp {
                    agent_id: b,
                    invocation_id: y,
                    ..
                },
            ) => a == b && x == y,
            (
                Self::AuthorityRefusal {
                    tool: tool_a,
                    reason_class: reason_a,
                    ..
                },
                Self::AuthorityRefusal {
                    tool: tool_b,
                    reason_class: reason_b,
                    ..
                },
            ) => tool_a == tool_b && reason_a == reason_b,
            _ => false,
        }
    }
}
