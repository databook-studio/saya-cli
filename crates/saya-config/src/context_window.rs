//! Model → context-window lookup.
//!
//! Most users reach a model through a gateway serving something this table will
//! never have heard of, so the lookup is exact-match and returns `None` for
//! anything it does not recognise. A wrong window is worse than no window: it
//! would either refuse work that would have succeeded, or promise headroom that
//! does not exist. Nothing consumes the answer yet — knowing a window and
//! acting on it are separate changes, and acting on a table that may be stale
//! breaks working setups.

/// Context window in tokens for a model identifier, matched exactly.
///
/// `None` means the table does not know the model — "not reported" rather than
/// a reported zero, the same discipline `TokenUsage` applies to cache counts.
/// A gateway model that is not in the table stays unknown; callers must behave
/// exactly as they do today when it is.
///
/// Names are matched exactly, never by prefix: `glm-5.2` and
/// `glm-5.2-1m-context` share a prefix and differ enormously, so a substring
/// relationship is not evidence of a shared window. Every entry names its
/// source, so a reviewer checks the claim rather than the code.
pub fn context_window_tokens(model: &str) -> Option<u64> {
    TABLE
        .iter()
        .find(|(name, _)| *name == model)
        .map(|(_, tokens)| *tokens)
}

/// `(model identifier, context window in tokens)`, one row per documented
/// model. Grouped by vendor, each group naming its source; within a group the
/// ids are the vendor's own, so a stale entry is found by checking the source
/// named above it.
static TABLE: &[(&str, u64)] = &[
    // Z.ai model pages (docs.z.ai/guides/llm, /guides/vlm), stated as a context
    // length per model. OpenRouter's `context_length` agrees for every entry
    // except the 5.3 pair, where it reports 1,310,720 — the vendor's own page
    // is the primary source and says 1M, so that is what is recorded here.
    ("glm-5.3", 1_048_576),
    ("glm-5.3-flash", 1_048_576),
    ("glm-5.2", 1_048_576),
    ("glm-5.1", 204_800),
    ("glm-5", 204_800),
    ("glm-4.7", 204_800),
    ("glm-4.6", 204_800),
    ("glm-4.5", 131_072),
    // OpenAI platform model pages (platform.openai.com/docs/models/<id>).
    ("gpt-4o", 128_000),
    ("gpt-4o-mini", 128_000),
    ("gpt-4.1", 1_047_576),
    ("gpt-4.1-mini", 1_047_576),
    ("gpt-4.1-nano", 1_047_576),
    ("gpt-5", 400_000),
    ("gpt-5-mini", 400_000),
    ("gpt-5-nano", 400_000),
    ("o3", 200_000),
    ("o3-mini", 200_000),
    ("o4-mini", 200_000),
    // Anthropic model pages (platform.claude.com/docs/en/models/<id>/overview):
    // 1M across the current lineup, 200K for the 4.5 generation and Haiku 4.5.
    ("claude-opus-5", 1_000_000),
    ("claude-sonnet-5", 1_000_000),
    ("claude-haiku-4-5", 200_000),
    ("claude-opus-4-5", 200_000),
    ("claude-sonnet-4-5", 200_000),
    // Google model pages (ai.google.dev/gemini-api/docs/models/<id>).
    ("gemini-2.5-pro", 1_048_576),
    ("gemini-2.5-flash", 1_048_576),
    ("gemini-2.5-flash-lite", 1_048_576),
    // Ollama library pages (ollama.com/library/<model>): the default serving
    // window a model starts with. A user may raise it with num_ctx, so this is
    // a conservative floor, not a promise about every deployment.
    ("qwen2.5-coder", 32_768),
    ("qwen2.5-coder:14b", 32_768),
    ("qwen2.5-coder:32b", 32_768),
    ("qwen3", 40_960),
    ("llama3.1", 128_000),
    ("llama3.1:8b", 128_000),
    ("llama3.1:70b", 128_000),
];

#[cfg(test)]
mod tests {
    use super::{TABLE, context_window_tokens};

    /// A zero window is a typo in the table, not a fact: the lookup's `None`
    /// means "unknown", and a table that could return `Some(0)` would make the
    /// two indistinguishable at a call site.
    #[test]
    fn no_entry_reports_a_zero_window() {
        for (_, tokens) in TABLE {
            assert!(*tokens > 0, "entry reports a zero window");
        }
    }

    /// A duplicated id is two rows disagreeing with each other; the later one
    /// would silently win. Nothing about a lookup tolerates that ambiguity.
    #[test]
    fn entries_are_unique_per_model() {
        let mut seen = Vec::new();
        for (model, _) in TABLE {
            assert!(
                !seen.contains(model),
                "duplicate table entry for {model}: {seen:?}"
            );
            seen.push(model);
        }
    }

    /// The lookup is total: every entry resolves to its own tokens, and a case
    /// difference is enough to be unknown.
    #[test]
    fn lookup_answers_for_every_input() {
        for (model, tokens) in TABLE {
            assert_eq!(
                context_window_tokens(model),
                Some(*tokens),
                "entry {model} must resolve to its own tokens"
            );
            assert_eq!(
                context_window_tokens(&model.to_uppercase()),
                None,
                "uppercased {model} must not match the lowercase entry"
            );
        }
    }
}
