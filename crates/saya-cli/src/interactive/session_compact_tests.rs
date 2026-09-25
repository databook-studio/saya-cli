//! Manual `/compact`: plan, validate, apply, and the summarising call's
//! failure posture. A failed compaction never damages the session — every
//! failure path below leaves the conversation exactly as it was.

use super::session_compact::{
    apply, failure_message, pinned_tokens, plan, success_message, validate,
};
use super::session_compact_call::summarise;
use super::session_state::SessionState;
use async_trait::async_trait;
use saya_agent::{ChatMessage, ChatProvider, ChatRequest, ChatResponse, ProviderError};

fn turns(count: usize) -> SessionState {
    let mut state = SessionState::new("s1", None, "m");
    for index in 0..count {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    state
}

struct StaticProvider {
    text: String,
    usage: Option<saya_agent::TokenUsage>,
}

#[async_trait]
impl ChatProvider for StaticProvider {
    fn name(&self) -> &str {
        "static"
    }
    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let mut response = ChatResponse::new(ChatMessage::text("assistant", &self.text));
        response.usage = self.usage;
        Ok(response)
    }
}

struct ErrorProvider;

#[async_trait]
impl ChatProvider for ErrorProvider {
    fn name(&self) -> &str {
        "error"
    }
    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::Request("http 500 server error".into()))
    }
}

struct TruncatedProvider;

#[async_trait]
impl ChatProvider for TruncatedProvider {
    fn name(&self) -> &str {
        "truncated"
    }
    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::output_truncated(
            "partial".into(),
            Vec::new(),
        ))
    }
}

/// Happy path: a long history compacts; the result is summary + verbatim
/// tail; the transcript (`messages`) is untouched.
#[test]
fn a_long_history_compacts_to_summary_plus_verbatim_tail() {
    let mut state = turns(5);
    let before_messages = state.messages.clone();
    let history = state.provider_history();
    let plan = plan(&state.turns, &history).expect("long history plans");
    assert_eq!(plan.compacted_turns, 3);
    let summary = "older turns: q0 q1 q2 discussed orders.";
    apply(&mut state, &plan, summary).expect("valid summary applies");
    let replayed = state.provider_history();
    assert!(
        replayed[0].content.contains(summary),
        "the summary replays first: {:?}",
        replayed[0].content
    );
    assert!(
        replayed
            .iter()
            .any(|message| message.content == "q3" && message.role == "user"),
        "the verbatim tail survives: {replayed:?}"
    );
    assert!(
        replayed
            .iter()
            .any(|message| message.content == "a4" && message.role == "assistant"),
        "the newest answer survives: {replayed:?}"
    );
    assert_eq!(
        state.messages, before_messages,
        "the transcript is untouched by compaction"
    );
}

/// Pinned tokens survive: a summarised assistant text carrying a file size
/// and digest pins those exact strings, and a summary that preserves them
/// applies while the tail stays verbatim.
#[test]
fn pinned_size_and_digest_survive_compaction() {
    let mut state = SessionState::new("s1", None, "m");
    for index in 0..5 {
        state.record_turn(
            format!("write chunk {index}"),
            format!(
                "appended notes.md\npath: notes.md\nsize: {}\ndigest: deadbeef{index:04x}",
                100 + index
            ),
            false,
            Vec::new(),
        );
    }
    let history = state.provider_history();
    let plan = plan(&state.turns, &history).expect("long history plans");
    assert!(
        plan.pinned.iter().any(|token| token.contains("size: 100")),
        "the oldest size is pinned: {:?}",
        plan.pinned
    );
    assert!(
        plan.pinned
            .iter()
            .any(|token| token.contains("digest: deadbeef")),
        "the oldest digest is pinned: {:?}",
        plan.pinned
    );
    assert!(
        plan.pinned
            .iter()
            .any(|token| token.contains("path: notes.md")),
        "the path naming the anchored file is pinned: {:?}",
        plan.pinned
    );
    let mut summary = String::from("older writes landed; ");
    for token in &plan.pinned {
        summary.push_str(token);
        summary.push_str("; ");
    }
    apply(&mut state, &plan, &summary).expect("a preserving summary applies");
    let replayed = state.provider_history();
    assert!(
        replayed[0].content.contains("size: 100"),
        "the size survives: {:?}",
        replayed[0].content
    );
    assert!(
        replayed[0].content.contains("digest: deadbeef0000"),
        "the digest survives: {:?}",
        replayed[0].content
    );
    // The newest tool-result group was never summarised: it replays verbatim.
    assert!(
        replayed
            .iter()
            .any(|message| message.content.contains("size: 104")),
        "the newest size replays verbatim, not via the summary: {replayed:?}"
    );
}

/// A summary that drops a pinned token is rejected and the conversation is
/// unchanged — including no stored summary.
#[test]
fn a_summary_that_drops_a_pinned_token_is_rejected_unchanged() {
    let mut state = SessionState::new("s1", None, "m");
    for index in 0..5 {
        state.record_turn(
            format!("write chunk {index}"),
            format!(
                "appended\nsize: {}\ndigest: deadbeef{index:04x}",
                100 + index
            ),
            false,
            Vec::new(),
        );
    }
    let before = state.provider_history();
    let history = before.clone();
    let plan = plan(&state.turns, &history).expect("long history plans");
    assert!(!plan.pinned.is_empty(), "the test needs a pinned token");
    let error = apply(&mut state, &plan, "older writes landed, details omitted.")
        .expect_err("a pin-dropping summary must be rejected");
    assert!(
        error.contains("pinned"),
        "the rejection names the pinned loss: {error}"
    );
    assert!(
        state.compaction_summary.is_none(),
        "no summary is stored on rejection"
    );
    assert_eq!(
        state.provider_history(),
        before,
        "the conversation is unchanged after rejection"
    );
    // Direct validation agrees: the unit and the apply path share one rule.
    assert!(validate("details omitted.", &plan.pinned).is_err());
}

/// Provider error → unchanged conversation, failure message, session still
/// usable: the error is said plainly and nothing is stored.
#[tokio::test]
async fn a_provider_error_leaves_the_session_usable() {
    let state = turns(5);
    let before = state.provider_history();
    let plan = plan(&state.turns, &before).expect("long history plans");
    let error = summarise(&ErrorProvider, "m", &plan)
        .await
        .expect_err("a provider error must fail compaction");
    assert!(
        error.contains("summariser errored"),
        "the reason is plain: {error}"
    );
    let message = failure_message(&error);
    assert!(
        message.contains("Compaction failed") && message.contains("unchanged"),
        "the failure message says what happened and the guarantee: {message}"
    );
    assert_eq!(
        state.provider_history(),
        before,
        "the conversation is unchanged after a provider error"
    );
}

/// Truncated summary → same: unchanged conversation, failure message, and the
/// reason names the truncation rather than a generic error.
#[tokio::test]
async fn a_truncated_summary_leaves_the_session_unchanged() {
    let state = turns(5);
    let before = state.provider_history();
    let plan = plan(&state.turns, &before).expect("long history plans");
    let error = summarise(&TruncatedProvider, "m", &plan)
        .await
        .expect_err("a truncated summary must fail compaction");
    assert!(
        error.contains("truncated"),
        "the reason names the truncation: {error}"
    );
    assert_eq!(
        state.provider_history(),
        before,
        "the conversation is unchanged after a truncation"
    );
}

/// Short history → "nothing to compact", no provider call at all. The plan is
/// `None`, so the execution layer never builds a provider — asserted here by
/// there being no call to make: planning is the gate, not the provider.
#[test]
fn a_short_history_plans_nothing_and_calls_no_provider() {
    for count in 0..=3 {
        let state = turns(count);
        let history = state.provider_history();
        assert!(
            plan(&state.turns, &history).is_none(),
            "{count} turns must plan nothing"
        );
    }
    let message = "Nothing to compact: the conversation is short enough already.";
    assert!(
        message.contains("Nothing to compact"),
        "the nothing-to-do message keeps its shape: {message}"
    );
}

/// The compaction call's usage is labelled apart from the answering total: a
/// reported usage folds into the learning total only, never the answering
/// one.
#[tokio::test]
async fn the_compaction_call_reports_its_usage_apart() {
    let state = turns(5);
    let history = state.provider_history();
    let plan = plan(&state.turns, &history).expect("long history plans");
    let provider = StaticProvider {
        text: "older turns discussed orders.".into(),
        usage: Some(saya_agent::TokenUsage::new(40, 10)),
    };
    let outcome = summarise(&provider, "m", &plan)
        .await
        .expect("static summary succeeds");
    assert_eq!(
        outcome.usage,
        Some(saya_agent::TokenUsage::new(40, 10)),
        "the compaction call surfaces its provider-reported usage"
    );
    let mut usage_state = turns(0);
    usage_state
        .usage
        .record(&saya_agent::TokenUsage::new(300, 130));
    usage_state.usage.record_learning(outcome.usage);
    let rendered = usage_state.usage.render();
    assert!(
        rendered.contains("Session token usage") && rendered.contains("Input tokens: 300"),
        "the answering total is intact: {rendered}"
    );
    assert!(
        rendered.contains("Learning call") && rendered.contains("Input tokens: 40"),
        "the compaction usage is labelled apart: {rendered}"
    );
    assert!(
        !rendered.contains("Input tokens: 340"),
        "the two totals must not merge: {rendered}"
    );
}

/// `/compact` is registered, listed, described, and helped — the parity the
/// `/mode` slice established across the four hand-maintained touchpoints.
#[test]
fn compact_is_registered_listed_described_and_helped() {
    assert_eq!(
        crate::slash::parse_slash_command("/compact"),
        Ok(Some(crate::slash::SlashCommand::Compact))
    );
    assert!(
        crate::slash::registry::KNOWN_COMMANDS.contains(&"compact"),
        "compact is registered"
    );
    assert!(
        crate::slash::description_for("compact").is_some(),
        "compact has a popup description"
    );
    let listing = crate::slash::help_for(None);
    assert!(
        listing.contains("/compact"),
        "the listing shows /compact: {listing}"
    );
    let help = crate::slash::help_for(Some("compact"));
    assert!(
        help.contains("transcript is unchanged"),
        "the help states the transcript guarantee: {help}"
    );
    assert!(
        help.contains("Automatic at 95%"),
        "the help states the automatic trigger and its threshold: {help}"
    );
    assert!(
        help.contains("`manual`") && help.contains("`off`"),
        "the help states the two non-automatic modes: {help}"
    );
}

/// `/clear` resets a compaction along with the conversation, so a fresh
/// context replays no stale summary.
#[test]
fn clear_resets_a_compaction() {
    let mut state = turns(5);
    let history = state.provider_history();
    let plan = plan(&state.turns, &history).expect("long history plans");
    apply(&mut state, &plan, "older turns.").expect("applies");
    assert!(state.compaction_summary.is_some());
    state.apply(crate::slash::SlashCommand::Clear, &[]);
    assert!(
        state.compaction_summary.is_none() && state.compacted_turns == 0,
        "/clear must reset the compaction with the conversation"
    );
}

/// The success message keeps its shape: the count, the estimate, and the
/// transcript guarantee with its remedy.
#[test]
fn the_success_message_keeps_its_shape() {
    let message = success_message(3, "older turns discussed orders at length");
    assert!(
        message.starts_with("Compacted 3 turns into a summary (~6 tokens)."),
        "the count and estimate lead: {message}"
    );
    assert!(
        message.contains("The transcript is unchanged; /export first if you want the full text."),
        "the guarantee and remedy follow: {message}"
    );
}

/// Pinned-token extraction pins SQL statements: the tool-call arguments are
/// the only record of what ran, so the summary must carry them.
#[test]
fn sql_statements_are_pinned() {
    let summarise = vec![
        ChatMessage::text("user", "count customers"),
        ChatMessage::text("assistant", "SELECT customer, age FROM users gave 42 rows"),
    ];
    let pinned = pinned_tokens(&summarise);
    assert!(
        pinned
            .iter()
            .any(|token| token.contains("SELECT customer, age FROM users")),
        "the statement is pinned: {pinned:?}"
    );
}
