//! The four turn-policy dimensions a plugin may state, and the merge the
//! registry applies to them.

use super::*;
use crate::AnsweringLimits;

/// Every dimension merges to the tighter of the two: a dimension one plugin
/// leaves unset keeps what the other stated, and the looser value never wins.
#[test]
fn turn_policy_takes_the_tightest_of_all_four_dimensions() {
    let generous = FakePlugin::new("generous", true).policy(TurnPolicy {
        max_requests: Some(6),
        max_tool_calls: Some(40),
        max_known_tokens: Some(9_000),
        max_elapsed: Some(Duration::from_secs(90)),
    });
    let strict = FakePlugin::new("strict", true).policy(TurnPolicy {
        max_requests: Some(4),
        max_tool_calls: None,
        max_known_tokens: Some(2_000),
        max_elapsed: None,
    });
    let registry = PluginRegistry::new(vec![Arc::new(generous), Arc::new(strict)], &[])
        .expect("distinct names");

    assert_eq!(
        registry.active(&CHAT).turn_policy(),
        Some(TurnPolicy {
            max_requests: Some(4),
            max_tool_calls: Some(40),
            max_known_tokens: Some(2_000),
            max_elapsed: Some(Duration::from_secs(90)),
        })
    );
}

/// Absence constrains nothing: a dimension only one plugin sets survives in
/// either registration order, and a dimension no plugin sets stays unset.
#[test]
fn turn_policy_keeps_a_dimension_only_one_plugin_states() {
    let requests = FakePlugin::new("requests", true).policy(TurnPolicy {
        max_requests: Some(5),
        ..Default::default()
    });
    let tools = FakePlugin::new("tools", true).policy(TurnPolicy {
        max_tool_calls: Some(8),
        ..Default::default()
    });
    let tokens = FakePlugin::new("tokens", true).policy(TurnPolicy {
        max_known_tokens: Some(1_000),
        ..Default::default()
    });
    let registry = PluginRegistry::new(
        vec![Arc::new(requests), Arc::new(tools), Arc::new(tokens)],
        &[],
    )
    .expect("distinct names");

    assert_eq!(
        registry.active(&CHAT).turn_policy(),
        Some(TurnPolicy {
            max_requests: Some(5),
            max_tool_calls: Some(8),
            max_known_tokens: Some(1_000),
            max_elapsed: None,
        })
    );
}

/// A plugin can state its whole answering table at once: the projection carries
/// all four dimensions, and a ceiling wider than the policy carries saturates
/// tighter rather than becoming unlimited.
#[test]
fn a_policy_from_answering_states_all_four_dimensions() {
    let stated = TurnPolicy::from_answering(
        AnsweringLimits {
            max_requests: Some(24),
            max_tool_calls: Some(64),
            max_reported_tokens: Some(250_000),
        },
        Some(Duration::from_secs(300)),
    );
    assert_eq!(
        stated,
        TurnPolicy {
            max_requests: Some(24),
            max_tool_calls: Some(64),
            max_known_tokens: Some(250_000),
            max_elapsed: Some(Duration::from_secs(300)),
        }
    );

    assert_eq!(
        TurnPolicy::from_answering(AnsweringLimits::default(), None),
        TurnPolicy::default(),
        "an unlimited surface asks for nothing"
    );
    assert_eq!(
        TurnPolicy::from_answering(
            AnsweringLimits {
                max_requests: Some(u64::from(u32::MAX) + 1),
                ..Default::default()
            },
            None,
        )
        .max_requests,
        Some(u32::MAX),
        "a ceiling wider than a policy dimension saturates tighter"
    );
}
