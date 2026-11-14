pub(crate) fn approval_name(policy: ApprovalPolicy) -> String {
    match policy {
        ApprovalPolicy::Ask => "ask",
        ApprovalPolicy::AcceptEdits => "accept-edits",
        ApprovalPolicy::ReadOnly => "read-only",
        ApprovalPolicy::Never => "never",
        ApprovalPolicy::Bypass => "bypass",
    }
    .into()
}

/// The `/mode` answer for a mode: the mode's own name, what it means, and —
/// for `plan` — that the model's tool list catches up at the next turn. Every
/// answer carries the shared bypass-composition sentence (see
/// [`crate::slash::PLAN_BYPASS_SENTENCE`]) in the same words as the detailed
/// help entry.
pub(crate) fn mode_message(mode: PermissionMode) -> String {
    let meaning = match mode {
        PermissionMode::Ask => "writes allowed, asks per approval policy",
        PermissionMode::AcceptEdits => {
            "workspace file writes run unasked; every other call still asks"
        }
        PermissionMode::Plan => {
            "read-only; write-shaped tools refuse now and leave the model's tool list at the next turn"
        }
        PermissionMode::Bypass => {
            "every tool call runs without asking; every structural guard still applies"
        }
    };
    format!(
        "Mode: {} — {meaning}. {}",
        crate::interactive::session::permissions::mode::name(mode),
        crate::slash::PLAN_BYPASS_SENTENCE
    )
}

/// The bare `/mode` report: the session's current mode, from the pair the
/// state carries. A consent posture with no mode of its own (`read-only`,
/// `never`, set by `/approvals`) is reported as the pair it is, never
/// rounded to one of the four.
pub(crate) fn mode_report(state: &SessionState) -> String {
    let policy = state
        .approval_mode_parsed()
        .unwrap_or(ApprovalPolicy::Ask);
    match PermissionMode::of(policy, state.agent_mode_parsed()) {
        Some(mode) => mode_message(mode),
        None => format!(
            "Mode: {} — the approval policy is {}, which is not one of the switchable modes \
             (ask, accept-edits, plan, bypass). {}",
            state.agent_mode_parsed().as_str(),
            approval_name(policy),
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
