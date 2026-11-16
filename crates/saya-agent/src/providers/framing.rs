use crate::{ProviderError, TokenUsage, protocol::stream_item_limit::StreamItemCount};
use std::time::Duration;

/// Bytes per token used to estimate how much visible output a cut stream had
/// produced. Reasoning is discarded, so it is not part of this measure.
pub(crate) const ESTIMATE_BYTES_PER_TOKEN: f64 = 3.5;

/// Tokens the visible `bytes` a cut stream delivered correspond to, rounded
/// up so a non-empty visible flow never estimates as zero.
pub(crate) fn estimate_tokens(bytes: usize) -> u32 {
    let estimate = (bytes as f64 / ESTIMATE_BYTES_PER_TOKEN).ceil();
    estimate.min(f64::from(u32::MAX)) as u32
}

/// The typed cut for a stream that outlived its total deadline. A deadline is
/// a clock: the visible bytes it delivered are reported, but they never size
/// the next output ceiling.
pub(crate) fn request_cut(elapsed: Duration, visible_bytes: usize) -> ProviderError {
    ProviderError::StreamCut {
        elapsed_secs: elapsed.as_secs().min(u64::from(u32::MAX)) as u32,
        estimated_tokens: estimate_tokens(visible_bytes),
    }
}

pub(super) fn whitespace(value: &[u8]) -> bool {
    value.iter().all(u8::is_ascii_whitespace)
}

pub(super) fn boundary(value: &[u8]) -> Option<(usize, usize)> {
    value.windows(2).enumerate().find_map(|(index, pair)| {
        if pair == b"\n\n" {
            Some((index, 2))
        } else if value
            .get(index..index.saturating_add(4))
            .is_some_and(|part| part == b"\r\n\r\n")
        {
            Some((index, 4))
        } else {
            None
        }
    })
}

#[derive(Default)]
pub(super) struct FrameCount(StreamItemCount);

impl FrameCount {
    pub(super) fn admit(&mut self, usage: Option<TokenUsage>) -> Result<(), ProviderError> {
        self.0.admit(usage)
    }
}
