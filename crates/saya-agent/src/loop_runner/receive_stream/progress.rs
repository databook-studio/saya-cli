//! The content-progress clock for one provider stream.
//!
//! The request deadline bounds the whole attempt. A gateway can hold that
//! attempt open with keep-alives — SSE comments the parser drops, empty
//! deltas, usage-only frames — while the model produces nothing, so the loop
//! measures the gap between content-bearing events instead. It is the same
//! window the streaming providers apply to the wire, applied again at the
//! boundary the loop owns, so a provider stream that is not one of those
//! drivers is bounded too.

use crate::ProviderEvent;
use std::time::Duration;

/// The window, and when it next expires, for a stream's content progress.
pub(super) struct ContentProgress {
    stall: Duration,
    at: tokio::time::Instant,
}

impl ContentProgress {
    /// The window runs from the stream's start, so a request that never
    /// produces anything is bounded too.
    pub(super) fn new(stall: Duration) -> Self {
        Self {
            stall,
            at: tokio::time::Instant::now() + stall,
        }
    }

    /// When a stream that stopped producing content is cut.
    pub(super) fn deadline(&self) -> tokio::time::Instant {
        self.at
    }

    /// Restarts the window when `event` carries content. Usage and signature
    /// frames are the shapes a gateway uses to keep a silent socket open, so
    /// they are not progress.
    pub(super) fn observe(&mut self, event: &ProviderEvent) {
        let progress = match event {
            ProviderEvent::TextDelta(text) | ProviderEvent::ReasoningDelta(text) => {
                !text.is_empty()
            }
            ProviderEvent::ToolCalls(calls) => !calls.is_empty(),
            ProviderEvent::ReasoningSignature(_) => true,
            ProviderEvent::Usage(_) | ProviderEvent::Done => false,
        };
        if progress {
            self.at = tokio::time::Instant::now() + self.stall;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ContentProgress;
    use crate::{ProviderEvent, TokenUsage};
    use std::time::Duration;

    /// Only content moves the window. The keep-alive shapes a gateway writes
    /// to hold a silent socket open must not.
    #[tokio::test(start_paused = true)]
    async fn keep_alive_frames_are_not_content_progress() {
        let mut progress = ContentProgress::new(Duration::from_secs(120));
        let start = progress.deadline();
        tokio::time::advance(Duration::from_secs(30)).await;
        progress.observe(&ProviderEvent::Usage(TokenUsage::default()));
        progress.observe(&ProviderEvent::TextDelta(String::new()));
        assert_eq!(progress.deadline(), start, "keep-alives do not move it");
        progress.observe(&ProviderEvent::TextDelta("a".into()));
        assert!(
            progress.deadline() > start,
            "a content delta restarts the window"
        );
    }
}
