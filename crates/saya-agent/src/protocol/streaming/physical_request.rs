//! Actual provider wire configuration, not a tokenizer estimate or grant.
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_CORRELATION_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_PROJECTION_REVISION: AtomicU64 = AtomicU64::new(1);

/// Locally assigned, opaque correlation identity. Pair it with the observation's
/// process and clock domain when comparing records across process runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CorrelationId(u64);

impl CorrelationId {
    pub(crate) fn next() -> Self {
        Self(NEXT_CORRELATION_ID.fetch_add(1, Ordering::Relaxed))
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

pub(crate) fn next_projection_revision() -> u64 {
    NEXT_PROJECTION_REVISION.fetch_add(1, Ordering::Relaxed)
}

/// A caller-supplied, content-free actor correlation digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActorCorrelationDigest([u8; 32]);

impl ActorCorrelationDigest {
    pub const fn from_bytes(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Content-free metadata that identifies one finalized request projection.
/// The revision is a process-local increasing snapshot sequence, not a digest
/// of the request contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalRequestCorrelation {
    conversation_id: CorrelationId,
    invocation_id: CorrelationId,
    actor_digest: Option<ActorCorrelationDigest>,
    request_ordinal: u64,
    projection_revision: u64,
    projection_size_bytes: usize,
    cache_key_digest: Option<u64>,
}

impl PhysicalRequestCorrelation {
    pub(crate) fn new(
        conversation_id: CorrelationId,
        invocation_id: CorrelationId,
        actor_digest: Option<ActorCorrelationDigest>,
        request_ordinal: u64,
        projection_revision: u64,
        projection_size_bytes: usize,
    ) -> Self {
        Self {
            conversation_id,
            invocation_id,
            actor_digest,
            request_ordinal,
            projection_revision,
            projection_size_bytes,
            cache_key_digest: None,
        }
    }

    pub const fn conversation_id(&self) -> CorrelationId {
        self.conversation_id
    }
    pub const fn invocation_id(&self) -> CorrelationId {
        self.invocation_id
    }
    pub const fn actor_digest(&self) -> Option<&ActorCorrelationDigest> {
        self.actor_digest.as_ref()
    }
    pub const fn request_ordinal(&self) -> u64 {
        self.request_ordinal
    }
    pub const fn projection_revision(&self) -> u64 {
        self.projection_revision
    }
    pub const fn projection_size_bytes(&self) -> usize {
        self.projection_size_bytes
    }
    pub const fn cache_key_digest(&self) -> Option<u64> {
        self.cache_key_digest
    }

    pub(crate) fn set_cache_key_digest(&mut self, digest: Option<u64>) {
        self.cache_key_digest = digest;
    }

    pub(crate) fn set_projection_size_bytes(&mut self, size: usize) {
        self.projection_size_bytes = size;
    }
}

#[derive(Clone)]
pub struct PhysicalRequestMetadata {
    provider: &'static str,
    model: String,
    endpoint: String,
    output_tokens: Option<u32>,
    correlation: Option<PhysicalRequestCorrelation>,
    thinking: Option<saya_types::ThinkingSpanData>,
    streamed: Option<bool>,
}
impl PhysicalRequestMetadata {
    /// Binds one finalized request projection. `output_tokens` is the ceiling
    /// the request asks the provider for, or `None` when it asks for none and
    /// the model's own default applies — absence is not zero.
    pub fn new(
        provider: &'static str,
        model: &str,
        endpoint: &str,
        output_tokens: Option<u32>,
    ) -> Result<Self, crate::EffectOwnerFailure> {
        if provider.is_empty()
            || provider.len() > 64
            || model.is_empty()
            || model.len() > 2048
            || endpoint.is_empty()
            || endpoint.len() > 8192
            || output_tokens == Some(0)
        {
            return Err(crate::EffectOwnerFailure::new(std::io::Error::other(
                "invalid physical request binding",
            )));
        }
        Ok(Self {
            provider,
            model: model.to_owned(),
            endpoint: endpoint.to_owned(),
            output_tokens,
            correlation: None,
            thinking: None,
            streamed: None,
        })
    }
    pub fn provider(&self) -> &str {
        self.provider
    }
    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub fn output_tokens(&self) -> Option<u32> {
        self.output_tokens
    }
    pub fn correlation(&self) -> Option<&PhysicalRequestCorrelation> {
        self.correlation.as_ref()
    }
    /// The thinking control the request actually sent, or `None` when it sent
    /// none. Absence is not a zero budget.
    pub fn thinking(&self) -> Option<saya_types::ThinkingSpanData> {
        self.thinking
    }
    /// The request's `stream` flag, or `None` when the route does not record
    /// one.
    pub fn streamed(&self) -> Option<bool> {
        self.streamed
    }

    /// Records the thinking control and stream flag the finalized body carries.
    /// Both are copied from the wire body, never inferred from configuration.
    pub fn with_thinking(
        mut self,
        thinking: Option<saya_types::ThinkingSpanData>,
        streamed: Option<bool>,
    ) -> Self {
        self.thinking = thinking;
        self.streamed = streamed;
        self
    }

    pub(crate) fn set_correlation(&mut self, correlation: Option<PhysicalRequestCorrelation>) {
        self.correlation = correlation;
    }
}
impl std::fmt::Debug for PhysicalRequestMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PhysicalRequestMetadata { endpoint: [redacted] }")
    }
}
