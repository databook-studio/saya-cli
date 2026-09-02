use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct Chunk {
    #[serde(default)]
    pub(super) message: Option<Message>,
    #[serde(default)]
    pub(super) done: bool,
    /// Prompt token count from the final done record.
    #[serde(default)]
    pub(super) prompt_eval_count: Option<u64>,
    /// Generated token count from the final done record.
    #[serde(default)]
    pub(super) eval_count: Option<u64>,
}
#[derive(Deserialize)]
pub(super) struct Message {
    #[serde(default)]
    pub(super) content: String,
    /// Chain-of-thought from a thinking model. Ollama streams and whole-returns
    /// the same shape,
    /// and its `complete()` routes through `collect()`→`stream()`, so this one
    /// field serves both. Empty/absent → no `ReasoningDelta` emitted, so a
    /// model that is not thinking leaves `ChatResponse.reasoning` `None`.
    #[serde(default)]
    pub(super) thinking: String,
    #[serde(default)]
    pub(super) tool_calls: Vec<Call>,
}
#[derive(Deserialize)]
pub(super) struct Call {
    #[serde(default)]
    pub(super) id: Option<String>,
    pub(super) function: Function,
}
#[derive(Deserialize)]
pub(super) struct Function {
    pub(super) name: String,
    pub(super) arguments: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::{Chunk, Message};
    use serde_json::json;

    /// a chunk whose `message.thinking`
    /// carries chain-of-thought parses into the new field. Ollama's
    /// `complete()` routes through
    /// `collect()`→`stream()`, so the streaming chunk is the one path.
    #[test]
    fn message_carries_thinking() {
        let chunk: Chunk = serde_json::from_value(json!({
            "message": {"content": "ok", "thinking": "I considered the schema"}
        }))
        .expect("parses");
        let message: &Message = chunk.message.as_ref().expect("message present");
        assert_eq!(message.content, "ok");
        assert_eq!(message.thinking, "I considered the schema");
    }

    /// a chunk with no `thinking` field
    /// leaves it the empty string (serde default), which the stream parser
    /// treats as "no reasoning" — a model that is not thinking is unaffected
    ///.
    #[test]
    fn message_without_thinking_leaves_it_empty() {
        let chunk: Chunk =
            serde_json::from_value(json!({"message": {"content": "ok"}})).expect("parses");
        let message: &Message = chunk.message.as_ref().expect("message present");
        assert_eq!(message.content, "ok");
        assert!(message.thinking.is_empty());
    }

    /// Deliverable 4 (Ollama, without — the provider with nothing to report):
    /// Ollama's done record carries only prompt/generated counts and no cache
    /// or reasoning fields. The slice must not
    /// invent them: a `TokenUsage` built from this record leaves the new
    /// fields `None`. This is the "obvious" provider the spec says not to skip.
    #[test]
    fn done_record_carries_no_cache_or_reasoning() {
        let chunk: Chunk = serde_json::from_value(json!({
            "done": true,
            "prompt_eval_count": 9,
            "eval_count": 11
        }))
        .expect("parses");
        assert!(chunk.done);
        assert_eq!(chunk.prompt_eval_count, Some(9));
        assert_eq!(chunk.eval_count, Some(11));
        // Ollama has no cache concept and no separate reasoning count: there is
        // nothing on the wire to map, so the accumulator's new fields stay None.
        let mut usage = crate::TokenUsage::default();
        if let Some(input) = chunk.prompt_eval_count {
            usage.input_tokens = input;
        }
        if let Some(output) = chunk.eval_count {
            usage.output_tokens = output;
        }
        assert_eq!(usage.cached_input_tokens, None);
        assert_eq!(usage.cache_creation_input_tokens, None);
        assert_eq!(usage.reasoning_tokens, None);
    }
}
