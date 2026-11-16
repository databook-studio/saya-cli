//! Durable child lineage references; these records never grant capability.
use super::{InvocationId, ManagedCheckoutId, Sha256Digest};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;

pub const MAX_CHILD_FOLLOW_UPS: u8 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum ChildInvocationRecord {
    ScopeBound {
        checkout_id: ManagedCheckoutId,
        scope_sha256: Sha256Digest,
    },
    FollowUpPrepared {
        checkout_id: ManagedCheckoutId,
        previous_invocation_id: InvocationId,
        previous_completion_sequence: NonZeroU64,
        source_sha256: Sha256Digest,
        scope_sha256: Sha256Digest,
        instruction_sha256: Sha256Digest,
        follow_up_index: u8,
    },
    FollowUpAdopted {
        prepared_sequence: NonZeroU64,
        source_sha256: Sha256Digest,
        instruction_sha256: Sha256Digest,
    },
}
