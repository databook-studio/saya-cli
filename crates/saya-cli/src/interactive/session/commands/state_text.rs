fn approval_name(policy: ApprovalPolicy) -> String {
    match policy {
        ApprovalPolicy::Ask => "ask",
        ApprovalPolicy::ReadOnly => "read-only",
        ApprovalPolicy::Never => "never",
        ApprovalPolicy::Bypass => "bypass",
    }
    .into()
}

/// The `/mode` answer: the session's current posture, with what it means.
/// Set when the command named a mode, reported either way — the `Approvals`
/// arm's shape, so the two sibling commands read alike. Both answers carry
/// the shared bypass-composition sentence (see
/// [`crate::slash::PLAN_BYPASS_SENTENCE`]) in the same words as the
/// detailed help entry.
fn mode_message(mode: &str) -> String {
    match mode {
        "plan" => format!(
            "Mode: plan — read-only; write-shaped tools are hidden and refuse. {}",
            crate::slash::PLAN_BYPASS_SENTENCE
        ),
        _ => format!(
            "Mode: build — writes allowed, asks per approval policy. {}",
            crate::slash::PLAN_BYPASS_SENTENCE
        ),
    }
}

fn available_providers() -> &'static [&'static str] {
    &[
        "ollama",
        "openai",
        "openai_compatible",
        "anthropic",
        "gemini",
    ]
}

/// Curated suggestions for models per provider (convenience suggestions, not an exhaustive or validated list).
fn known_models(provider: &str) -> &'static [&'static str] {
    match provider.to_ascii_lowercase().as_str() {
        "ollama" => &["qwen2.5-coder:14b", "llama3.1", "mistral"],
        "openai" => &["gpt-4o", "gpt-4o-mini", "o3-mini"],
        "anthropic" => &["claude-sonnet-4", "claude-opus-4", "claude-3-5-haiku"],
        "gemini" => &["gemini-2.0-flash", "gemini-1.5-pro"],
        "openai_compatible" => &[],
        _ => &[],
    }
}
