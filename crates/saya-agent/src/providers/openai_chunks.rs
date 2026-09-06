use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct Chunk {
    pub(super) choices: Vec<Choice>,
    #[serde(default)]
    pub(super) usage: Option<Usage>,
}
#[derive(Deserialize)]
pub(super) struct Choice {
    pub(super) delta: Delta,
    #[serde(default)]
    pub(super) finish_reason: Option<String>,
}
#[derive(Deserialize, Default)]
pub(super) struct Delta {
    #[serde(default)]
    pub(super) content: Option<String>,
    /// Chain-of-thought from a reasoning model. OpenAI-compatible gateways
    /// spell this `delta.reasoning_content` (the field the `glm-5.2`
    /// measurement found on every response); some spell it `reasoning`, so
    /// both are accepted. Streamed only — OpenAI's `complete()` routes
    /// through `collect()` and drives a `stream: true` request, so there is no
    /// whole-response `message.reasoning_content` path to parse here.
    #[serde(default, alias = "reasoning")]
    pub(super) reasoning_content: Option<String>,
    #[serde(default)]
    pub(super) tool_calls: Vec<Call>,
}
#[derive(Deserialize)]
pub(super) struct Call {
    pub(super) index: usize,
    #[serde(default)]
    pub(super) id: Option<String>,
    pub(super) function: Function,
}
#[derive(Deserialize, Default)]
pub(super) struct Function {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) arguments: Option<String>,
}

/// Token counts from the trailing usage-only chunk requested via
/// `stream_options.include_usage`.
#[derive(Deserialize, Default)]
pub(super) struct Usage {
    #[serde(default, alias = "input_tokens")]
    pub(super) prompt_tokens: Option<u64>,
    #[serde(default, alias = "output_tokens")]
    pub(super) completion_tokens: Option<u64>,
    /// OpenAI `usage.prompt_tokens_details`. Present only when the gateway
    /// reports prompt-token breakdowns (e.g. `cached_tokens`).
    #[serde(default, alias = "input_tokens_details")]
    pub(super) prompt_tokens_details: Option<PromptTokensDetails>,
    /// OpenAI `usage.completion_tokens_details`. Present only when the gateway
    /// reports completion-token breakdowns (e.g. `reasoning_tokens`).
    #[serde(default, alias = "output_tokens_details")]
    pub(super) completion_tokens_details: Option<CompletionTokensDetails>,
}

/// OpenAI `usage.prompt_tokens_details`: the prompt side, including cache hits.
#[derive(Deserialize, Default, Debug, PartialEq)]
pub(super) struct PromptTokensDetails {
    /// `prompt_tokens_details.cached_tokens` — prompt tokens served from
    /// cache. Inclusive of `prompt_tokens`. `Some(0)` is a reported cache
    /// miss; absent is `None`.
    #[serde(default)]
    pub(super) cached_tokens: Option<u64>,
}

/// OpenAI `usage.completion_tokens_details`: the completion side, including
/// reasoning.
#[derive(Deserialize, Default, Debug, PartialEq)]
pub(super) struct CompletionTokensDetails {
    /// `completion_tokens_details.reasoning_tokens` — chain-of-thought
    /// tokens. **Inclusive of `completion_tokens`** (inside it), so adding the
    /// two double-counts.
    #[serde(default)]
    pub(super) reasoning_tokens: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::{Delta, Usage};
    use serde_json::json;

    /// a streaming
    /// delta carrying `delta.reasoning_content` parses into the new field,
    /// ready to become a `ReasoningDelta` event.
    #[test]
    fn delta_carries_reasoning_content() {
        let delta: Delta = serde_json::from_value(json!({
            "reasoning_content": "I considered the time column"
        }))
        .expect("parses");
        assert_eq!(
            delta.reasoning_content.as_deref(),
            Some("I considered the time column")
        );
    }

    ///
    /// some OpenAI-compatible gateways spell it `reasoning` rather than
    /// `reasoning_content`. The `alias` accepts both.
    #[test]
    fn delta_accepts_reasoning_alias() {
        let delta: Delta =
            serde_json::from_value(json!({"reasoning": "alt spelling"})).expect("parses");
        assert_eq!(delta.reasoning_content.as_deref(), Some("alt spelling"));
    }

    /// a delta with no reasoning field
    /// leaves `reasoning_content` `None`, and content still parses — a
    /// non-reasoning response is unaffected.
    #[test]
    fn delta_without_reasoning_leaves_it_none() {
        let delta: Delta = serde_json::from_value(json!({"content": "ok"})).expect("parses");
        assert_eq!(delta.reasoning_content, None);
        assert_eq!(delta.content.as_deref(), Some("ok"));
    }

    /// Deliverable 4 (OpenAI, with): a usage chunk carrying both detail
    /// objects populates the new fields.
    #[test]
    fn usage_with_details_populates_cache_and_reasoning() {
        let usage: Usage = serde_json::from_value(json!({
            "prompt_tokens": 100,
            "completion_tokens": 200,
            "prompt_tokens_details": {"cached_tokens": 90},
            "completion_tokens_details": {"reasoning_tokens": 270}
        }))
        .expect("parses");
        assert_eq!(usage.prompt_tokens, Some(100));
        assert_eq!(usage.completion_tokens, Some(200));
        assert_eq!(
            usage
                .prompt_tokens_details
                .as_ref()
                .and_then(|d| d.cached_tokens),
            Some(90)
        );
        assert_eq!(
            usage
                .completion_tokens_details
                .as_ref()
                .and_then(|d| d.reasoning_tokens),
            Some(270)
        );
    }

    /// Deliverable 4 (OpenAI, without — the assertion that matters): a usage
    /// chunk with no detail objects leaves them `None`, and the two existing
    /// counters still parse. Also accepts the `input_tokens`/`output_tokens`
    /// aliases some OpenAI-compatible gateways use.
    #[test]
    fn usage_without_details_leaves_them_none() {
        let usage: Usage = serde_json::from_value(json!({
            "input_tokens": 5,
            "output_tokens": 6
        }))
        .expect("parses");
        assert_eq!(usage.prompt_tokens, Some(5));
        assert_eq!(usage.completion_tokens, Some(6));
        assert_eq!(usage.prompt_tokens_details, None);
        assert_eq!(usage.completion_tokens_details, None);
    }

    /// Deliverable 5 (OpenAI): a reported `cached_tokens: 0` deserialises to
    /// `Some(0)`, distinct from an omitted field (`None`).
    #[test]
    fn reported_zero_cached_tokens_is_some_zero_not_none() {
        let with_zero: Usage = serde_json::from_value(json!({
            "prompt_tokens": 10,
            "prompt_tokens_details": {"cached_tokens": 0}
        }))
        .expect("parses");
        assert_eq!(
            with_zero
                .prompt_tokens_details
                .as_ref()
                .and_then(|d| d.cached_tokens),
            Some(0)
        );
        let without: Usage = serde_json::from_value(json!({
            "prompt_tokens": 10,
            "prompt_tokens_details": {}
        }))
        .expect("parses");
        assert_eq!(
            without
                .prompt_tokens_details
                .as_ref()
                .and_then(|d| d.cached_tokens),
            None
        );
    }
}
