//! The turn-finish hook's usage contract: every active plugin reports what its
//! own post-turn work spent, and the registry hands the turn one summed total.

use std::sync::{Arc, Mutex};

use super::super::{PluginRegistry, TokenUsage};
use super::support::{CHAT, FakePlugin, answered};

/// A plugin's reported usage lands in the turn's total: `{3, 1}` and `{2, 2}`
/// sum to `{5, 3}`.
#[tokio::test]
async fn on_turn_finish_sums_the_active_plugins_usages() {
    let first = FakePlugin::new("first", true).usage(TokenUsage::new(3, 1));
    let second = FakePlugin::new("second", true).usage(TokenUsage::new(2, 2));
    let registry =
        PluginRegistry::new(vec![Arc::new(first), Arc::new(second)], &[]).expect("distinct names");

    let usage = registry
        .active(&CHAT)
        .on_turn_finish(&answered(), &crate::NoopEventSink)
        .await;

    assert_eq!(usage, Some(TokenUsage::new(5, 3)));
}

/// No active plugin reporting usage leaves the turn's total absent — absent is
/// not a reported zero.
#[tokio::test]
async fn on_turn_finish_is_none_when_no_plugin_reports_usage() {
    let silent = FakePlugin::new("silent", true);
    let registry = PluginRegistry::new(vec![Arc::new(silent)], &[]).expect("no collision");

    let usage = registry
        .active(&CHAT)
        .on_turn_finish(&answered(), &crate::NoopEventSink)
        .await;

    assert_eq!(usage, None);
}

/// An inactive plugin is never called, so its usage cannot reach the total.
#[tokio::test]
async fn on_turn_finish_skips_an_inactive_plugins_usage() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let active = FakePlugin::new("active", true)
        .usage(TokenUsage::new(1, 1))
        .logging(calls.clone());
    let inactive = FakePlugin::new("inactive", false)
        .usage(TokenUsage::new(9, 9))
        .logging(calls.clone());
    let registry =
        PluginRegistry::new(vec![Arc::new(active), Arc::new(inactive)], &[]).expect("distinct");

    let usage = registry
        .active(&CHAT)
        .on_turn_finish(&answered(), &crate::NoopEventSink)
        .await;

    assert_eq!(usage, Some(TokenUsage::new(1, 1)));
    assert_eq!(*calls.lock().unwrap(), vec!["active:finish"]);
}
