use crate::{ProviderError, TokenUsage, protocol::stream_item_limit::StreamItemCount};

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
