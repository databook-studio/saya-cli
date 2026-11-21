//! A plugin's section of the run's system prompt, as the registry collects it.

use std::sync::Arc;

use super::super::{PluginRegistry, RunPlugin};
use super::support::{CHAT, FakePlugin};

/// A plugin that states no section — the default — states none, and a registry
/// of such plugins has nothing to append after the core prompt.
#[test]
fn a_plugin_without_a_section_states_none() {
    let plain = FakePlugin::new("plain", true);
    assert_eq!(
        plain.system_section(&CHAT),
        None,
        "the default contributes no section"
    );

    let registry = PluginRegistry::new(vec![Arc::new(plain)], &[]).expect("no collision");
    assert!(
        registry.active(&CHAT).system_sections().is_empty(),
        "no plugin states a section, so the run's prompt gains nothing"
    );
}

/// The active plugins' sections in registration order: a plugin stating none is
/// skipped rather than contributing an empty one, and an inactive plugin's
/// section is never collected.
#[test]
fn sections_keep_registration_order_and_skip_silent_or_inactive_plugins() {
    let first = FakePlugin::new("first", true).section("the first section");
    let silent = FakePlugin::new("silent", true);
    let inactive = FakePlugin::new("inactive", false).section("never collected");
    let third = FakePlugin::new("third", true).section("the third section");
    let registry = PluginRegistry::new(
        vec![
            Arc::new(first),
            Arc::new(silent),
            Arc::new(inactive),
            Arc::new(third),
        ],
        &[],
    )
    .expect("distinct");

    assert_eq!(
        registry.active(&CHAT).system_sections(),
        vec![
            "the first section".to_string(),
            "the third section".to_string()
        ]
    );
}
