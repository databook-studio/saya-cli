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
/// help entry. One answer for the whole six-value axis: `/mode read-only` and
/// `/approvals read-only` read the same.
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
        PermissionMode::ReadOnly => {
            "read-shaped calls run; every call that would need approval is denied instead"
        }
        PermissionMode::Never => {
            "nothing is put to you: every call that would need approval is denied"
        }
    };
    format!(
        "Mode: {} — {meaning}. {}",
        crate::interactive::session::permissions::mode::name(mode),
        crate::slash::PLAN_BYPASS_SENTENCE
    )
}

/// The bare `/mode` report: the session's current mode, from the pair the
/// state carries. Every posture the pair can hold is one of the six modes, so
/// the report always names a mode — there is no second vocabulary to read
/// around.
pub(crate) fn mode_report(state: &SessionState) -> String {
    let policy = state
        .approval_mode_parsed()
        .unwrap_or(ApprovalPolicy::Ask);
    mode_message(PermissionMode::of(policy, state.agent_mode_parsed()))
}

/// One panel section plus its alias's note, as `/workspace`
/// (`Note::Workspace`, `Section::Workspace`) answers: the one panel builder,
/// so an alias can never disagree with `/status` about the fact it shows.
pub(crate) fn section_answer(
    state: &mut SessionState,
    note: crate::chat::ops_status::Note,
    section: crate::chat::ops_status::Section,
) -> SessionAction {
    let text = crate::chat::ops_status::StatusPanel::build(
        &crate::chat::ops_status::StatusFacts::of_session(state),
    )
    .render_section(section);
    SessionAction::Message(crate::chat::ops_status::with_note(state, note, text))
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
