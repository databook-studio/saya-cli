use super::contracts::ProviderError;
use crate::protocol::streaming::TokenUsage;

/// Maximum semantic provider events accepted from one response attempt.
pub const MAX_STREAM_EVENTS: usize = 65_536;

#[derive(Default)]
pub(crate) struct StreamItemCount(usize);

impl StreamItemCount {
    /// Counts the next semantic event or framed record before it is retained.
    pub(crate) fn admit(&mut self, usage: Option<TokenUsage>) -> Result<(), ProviderError> {
        self.0 = self
            .0
            .checked_add(1)
            .ok_or_else(|| ProviderError::StreamItemLimit {
                usage: usage.map(Box::new),
            })?;
        if self.0 > MAX_STREAM_EVENTS {
            return Err(ProviderError::StreamItemLimit {
                usage: usage.map(Box::new),
            });
        }
        Ok(())
    }
}
