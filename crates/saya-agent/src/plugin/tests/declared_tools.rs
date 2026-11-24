//! The collision check, the definition filter, and the route check that all
//! key on [`RunPlugin::declared_tools`] rather than on what a plugin happens to
//! offer for one probed context.

use std::sync::Arc;

use super::super::{PluginError, PluginRegistry};
use super::support::{CHAT, FakePlugin, NO_DB, definition_names};

/// Case 3c — a plugin that offers a tool only when a database profile exists is
/// refused when the name belongs to the core: the collision is checked against
/// every name the plugin can ever deliver, not against one probed context.
#[test]
fn a_tool_offered_only_with_a_profile_is_still_refused() {
    let plugin = FakePlugin::new("db", true)
        .profile_tools(&["db_query", "shell"])
        .serving("db");
    let error = PluginRegistry::new(vec![Arc::new(plugin)], &["shell"])
        .expect_err("the core owns shell, and the plugin can deliver it");
    assert_eq!(
        error,
        PluginError::NameCollision {
            plugin: "db",
            tool: "shell".to_string(),
        }
    );
}

/// Case 8 — a definition whose name the plugin never declared never reaches the
/// core.
///
/// A debug build asserts against the misfit instead of filtering it quietly, so
/// the call goes through `catch_unwind`: the assertion is what a debug run must
/// see, the filtered list is what a release run must produce.
#[test]
fn definitions_drop_a_name_the_plugin_never_declared() {
    let plugin = FakePlugin::new("misfit", true)
        .declaring(&["kept"])
        .offering(&["kept", "undeclared"]);
    let registry = PluginRegistry::new(vec![Arc::new(plugin)], &[]).expect("kept is declared");
    let active = registry.active(&CHAT);

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let called = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| active.definitions()));
    std::panic::set_hook(previous);

    if cfg!(debug_assertions) {
        assert!(
            called.is_err(),
            "a debug build asserts against the undeclared definition"
        );
    } else {
        assert_eq!(
            definition_names(called.expect("no assertion in a release build")),
            vec!["kept"],
            "the undeclared definition is dropped"
        );
    }
}

/// Case 9 — routing never leaves the plugin's declared names, and never serves
/// a declared tool the plugin does not offer in this context.
#[test]
fn route_returns_none_for_a_tool_this_context_does_not_offer() {
    let gated = FakePlugin::new("db", true)
        .profile_tools(&["db_query"])
        .serving("db");
    let stray = FakePlugin::new("stray", true)
        .declaring(&["stray_tool"])
        .offering(&["stray_tool", "undeclared_tool"])
        .serving("stray");
    let registry =
        PluginRegistry::new(vec![Arc::new(gated), Arc::new(stray)], &[]).expect("distinct names");
    let without_profile = registry.active(&NO_DB);
    let with_profile = registry.active(&CHAT);

    assert!(
        without_profile.route("db_query").is_none(),
        "declared, but offered only with a database profile"
    );
    assert!(
        with_profile.route("undeclared_tool").is_none(),
        "offered now, but never declared, so it is not the plugin's to route"
    );
    assert!(
        with_profile.route("db_query").is_some(),
        "declared and offered here, so the plugin answers for it"
    );
}
