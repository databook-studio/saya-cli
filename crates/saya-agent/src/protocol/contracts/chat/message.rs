use serde::{Deserialize, Serialize};

use super::image::ImageBlock;

/// Ceiling on the reasoning text carried back to a provider on the next
/// request. A model can emit far more chain-of-thought than fits a prompt; the
/// replay is a nudge, not a transcript, so the tail is dropped.
pub const MAX_REPLAYED_REASONING_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Images this message carries beside its text — the image a workspace
    /// `read` returned.
    ///
    /// Never serialized. The loop measures, trims, and replays messages as
    /// JSON text; an image is not text, and counting its bytes against the
    /// context byte budget would evict the very message that carries it. It is
    /// bounded by its own cap where it is read, and an adapter that cannot send
    /// it says so in the message's own text.
    #[serde(default, skip)]
    pub images: Vec<ImageBlock>,
    /// The chain-of-thought this assistant turn produced, kept only so the
    /// next request can replay it on this same message. `#[serde(skip)]` keeps
    /// it out of sessions and job records; the request projection measures it
    /// explicitly, because it is sent on the wire.
    /// Bounded to [`MAX_REPLAYED_REASONING_BYTES`] by [`Self::with_reasoning`].
    /// The loop keeps every assistant turn's copy — append-only, so the cached
    /// request prefix is never rewritten — and each copy is capped on its own.
    #[serde(default, skip)]
    pub reasoning: Option<String>,
    /// The provider's signature over [`Self::reasoning`], when the route
    /// issued one. Anthropic accepts a previous turn's thinking back only as
    /// a signed block; without a signature the text is kept for other routes
    /// but never replayed as thinking. `#[serde(skip)]` for the same reason
    /// as the text: it is transport for the next request, never history.
    /// Dropped whenever the text was truncated or the pair exceeds
    /// [`MAX_REPLAYED_REASONING_BYTES`] — see [`Self::with_reasoning_trace`].
    #[serde(default, skip)]
    pub reasoning_signature: Option<String>,
}

impl ChatMessage {
    pub fn text(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            images: Vec::new(),
            reasoning: None,
            reasoning_signature: None,
        }
    }

    /// The same message with `images` attached beside its text.
    pub fn with_images(mut self, images: Vec<ImageBlock>) -> Self {
        self.images = images;
        self
    }

    /// The same message carrying reasoning for the next request, truncated to
    /// [`MAX_REPLAYED_REASONING_BYTES`] at a UTF-8 boundary. `None` and the
    /// empty string both leave the field absent — a model that reasoned and
    /// produced nothing has nothing to replay.
    pub fn with_reasoning(mut self, reasoning: Option<String>) -> Self {
        self.reasoning = reasoning
            .filter(|text| !text.is_empty())
            .map(|text| cap_reasoning(&text));
        self
    }

    /// The same message carrying one signed thinking block. The signature is
    /// kept only when it describes the whole replayed text and both fit the
    /// bound; a truncated or oversized block would be rejected by the
    /// provider, so its signature is dropped and today's text-only replay
    /// remains.
    pub fn with_reasoning_trace(mut self, text: Option<String>, signature: Option<String>) -> Self {
        let full_len = text.as_ref().map_or(0, String::len);
        self = self.with_reasoning(text);
        let capped_len = self.reasoning.as_ref().map_or(0, String::len);
        self.reasoning_signature = signature.filter(|signature| {
            self.reasoning.is_some()
                && !signature.is_empty()
                && full_len == capped_len
                && capped_len.saturating_add(signature.len()) <= MAX_REPLAYED_REASONING_BYTES
        });
        self
    }
}

fn cap_reasoning(text: &str) -> String {
    if text.len() <= MAX_REPLAYED_REASONING_BYTES {
        return text.to_owned();
    }
    let mut end = MAX_REPLAYED_REASONING_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::{ChatMessage, MAX_REPLAYED_REASONING_BYTES};

    #[test]
    fn a_signed_trace_that_fits_keeps_both_halves() {
        let message = ChatMessage::text("assistant", "answer")
            .with_reasoning_trace(Some("plan".into()), Some("sig-1".into()));
        assert_eq!(message.reasoning.as_deref(), Some("plan"));
        assert_eq!(message.reasoning_signature.as_deref(), Some("sig-1"));
    }

    #[test]
    fn a_truncated_trace_keeps_the_capped_text_and_drops_the_signature() {
        let text = "x".repeat(MAX_REPLAYED_REASONING_BYTES + 1);
        let message = ChatMessage::text("assistant", "answer")
            .with_reasoning_trace(Some(text), Some("sig-1".into()));
        assert_eq!(
            message.reasoning.as_ref().map(String::len),
            Some(MAX_REPLAYED_REASONING_BYTES)
        );
        assert_eq!(
            message.reasoning_signature, None,
            "a signature over a truncated block is invalid"
        );
    }

    #[test]
    fn a_trace_whose_halves_exceed_the_bound_drops_the_signature() {
        let text = "x".repeat(MAX_REPLAYED_REASONING_BYTES / 2 + 1);
        let signature = "s".repeat(MAX_REPLAYED_REASONING_BYTES / 2);
        let message = ChatMessage::text("assistant", "answer")
            .with_reasoning_trace(Some(text), Some(signature));
        assert!(message.reasoning.is_some(), "the text still replays");
        assert_eq!(message.reasoning_signature, None);
    }

    #[test]
    fn an_unsigned_trace_replays_its_text_only() {
        let message = ChatMessage::text("assistant", "answer")
            .with_reasoning_trace(Some("plan".into()), None);
        assert_eq!(message.reasoning.as_deref(), Some("plan"));
        assert_eq!(message.reasoning_signature, None);
    }
}
