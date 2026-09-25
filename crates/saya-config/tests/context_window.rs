//! Model → context-window lookup.
//!
//! The table is a set of named models with documented windows; the lookup must
//! answer a known model exactly, refuse to guess for any other string, and let
//! a user-declared value stand even when the table also knows the model.

use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, context_window_tokens, resolve};

/// A model the table knows resolves to its documented window. `glm-5.2` is the
/// model the shipped example config runs on, so its entry is the one most often
/// asked for.
#[test]
fn known_model_returns_its_documented_window() {
    assert_eq!(context_window_tokens("glm-5.2"), Some(1_048_576));
}

/// A second family, from a different provider, resolves independently — the
/// table is not GLM-shaped by accident.
#[test]
fn another_known_family_resolves_too() {
    assert_eq!(context_window_tokens("qwen2.5-coder:14b"), Some(32_768));
}

/// Most users run through gateways serving models the table has never heard
/// of. The answer must be "unknown", never a number — a wrong window would
/// either refuse work that would have succeeded or promise headroom that does
/// not exist. A near-miss string must not match either.
#[test]
fn unknown_models_are_not_guessed() {
    assert_eq!(context_window_tokens("totally-made-up-model"), None);
    assert_eq!(context_window_tokens(""), None);
    assert_eq!(context_window_tokens("glm-5.2-plus"), None);
    assert_eq!(
        context_window_tokens("GLM-5.2"),
        None,
        "matching is on the exact string, not a case-folded one"
    );
}

/// The reason exact matching is the rule: these names share prefixes and differ
/// enormously. A prefix rule would answer 1M for all of them; the table must
/// distinguish the ones it knows and refuse the ones it does not.
///
/// `glm-5.2-1m-context` is deliberately absent: the name appears in no vendor
/// model page, only as a gateway alias, and there is no documented source for
/// its window. A table whose entries are all sourced cannot hold it — and the
/// user who runs it declares the window in `[ai] context_window_tokens`, which
/// is what that setting is for.
#[test]
fn models_sharing_a_prefix_do_not_share_a_window() {
    assert_eq!(context_window_tokens("glm-5.2"), Some(1_048_576));
    assert_eq!(context_window_tokens("glm-5.3"), Some(1_048_576));
    assert_eq!(context_window_tokens("glm-5.3-flash"), Some(1_048_576));
    assert_eq!(context_window_tokens("glm-5.2-1m-context"), None);
    assert_eq!(context_window_tokens("glm-5"), Some(204_800));
    assert_eq!(context_window_tokens("glm-4.5"), Some(131_072));
}

/// Every entry is a fact with a source; this pins the ones a wrong answer would
/// cost the most on — the 1M windows, where over-promising is the risk, and the
/// small local models, where under-quoting is. If any of these changes
/// upstream, this test is what fails first.
#[test]
fn windows_match_their_documented_sources() {
    // Z.ai model pages (docs.z.ai/guides/llm); OpenRouter's `context_length`
    // agrees everywhere except the 5.3 pair, which the vendor states as 1M.
    assert_eq!(context_window_tokens("glm-5.2"), Some(1_048_576));
    assert_eq!(context_window_tokens("glm-5.1"), Some(204_800));
    assert_eq!(context_window_tokens("glm-4.7"), Some(204_800));
    assert_eq!(context_window_tokens("glm-4.6"), Some(204_800));
    assert_eq!(context_window_tokens("glm-4.5"), Some(131_072));
    // OpenAI platform model pages.
    assert_eq!(context_window_tokens("gpt-4o"), Some(128_000));
    assert_eq!(context_window_tokens("gpt-4.1"), Some(1_047_576));
    assert_eq!(context_window_tokens("o3"), Some(200_000));
    assert_eq!(context_window_tokens("o3-mini"), Some(200_000));
    // Anthropic model pages: 1M for the current lineup, 200K for the
    // 4.5-generation Opus and Sonnet and for Haiku 4.5.
    assert_eq!(context_window_tokens("claude-opus-5"), Some(1_000_000));
    assert_eq!(context_window_tokens("claude-sonnet-5"), Some(1_000_000));
    assert_eq!(context_window_tokens("claude-opus-4-5"), Some(200_000));
    assert_eq!(context_window_tokens("claude-sonnet-4-5"), Some(200_000));
    assert_eq!(context_window_tokens("claude-haiku-4-5"), Some(200_000));
    // Google model pages.
    assert_eq!(context_window_tokens("gemini-2.5-pro"), Some(1_048_576));
    assert_eq!(context_window_tokens("gemini-2.5-flash"), Some(1_048_576));
}

/// `[ai] context_window_tokens` is a user declaration about their own
/// deployment — the one person who knows the window of a gateway-served model
/// the table will never list. It wins over the table: a user who states the
/// window is stating something about their gateway that the published fact for
/// the model name cannot know.
#[test]
fn a_config_declared_window_overrides_the_table() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_user(
            ConfigFile::from_toml("[ai]\nmodel = 'glm-5.2'\ncontext_window_tokens = 500000\n")
                .expect("known keys must parse"),
        ),
    )
    .expect("resolution succeeds");
    assert_eq!(resolved.ai.context_window_tokens, Some(500_000));
}

/// When the user declares nothing, the field stays `None` so the lookup can be
/// asked at the point of use — a known model's window is not resolved into the
/// config value, it stays a table question.
#[test]
fn resolution_leaves_the_field_absent_when_unconfigured() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[ai]\nmodel = 'glm-5.2'\n").expect("parses")),
    )
    .expect("resolution succeeds");
    assert_eq!(resolved.ai.context_window_tokens, None);

    let defaults = resolve(ResolutionInput::new(ConnectionsFile::default())).expect("resolves");
    assert_eq!(defaults.ai.context_window_tokens, None);
}

/// A declared `0` is not a window, it is a typo: every other numeric setting in
/// this crate rejects a meaningless value at resolve time rather than clamping
/// it at the point of use.
#[test]
fn a_declared_zero_window_is_rejected() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[ai]\ncontext_window_tokens = 0\n").expect("parses")),
    )
    .unwrap_err();
    let rendered = format!("{error:?}");
    assert!(
        rendered.contains("context_window_tokens"),
        "error must name the field: {rendered}"
    );
}

/// The table and the config field answer the same question, so an unknown model
/// must land on `None` through both — not `Some(0)` through one of them. If the
/// two paths ever disagree on the shape of "absent", a caller comparing them
/// will be wrong in exactly the way that is hardest to spot.
#[test]
fn the_lookup_and_the_resolved_field_agree_on_absence() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[ai]\nmodel = 'no-such-model'\n").expect("parses")),
    )
    .expect("resolution succeeds");
    assert_eq!(resolved.ai.context_window_tokens, None);
    assert_eq!(context_window_tokens("no-such-model"), None);
}
