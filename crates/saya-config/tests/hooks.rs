//! The `[hooks]` table: what it accepts, what it refuses, and what a resolved
//! hook is.

use saya_config::{
    ConfigFile, ConnectionsFile, HookEvent, MAX_HOOK_TIMEOUT_MS, MAX_HOOKS_PER_EVENT,
    ResolutionInput, resolve,
};

fn resolved(toml: &str) -> saya_config::ResolvedConfig {
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_user(ConfigFile::from_toml(toml).expect("fixture must parse"));
    resolve(input).expect("resolution succeeds")
}

#[test]
fn the_three_events_resolve_in_order() {
    let resolved = resolved(
        "[[hooks.pre_tool_use]]\ncommand = \"one\"\n\n\
         [[hooks.pre_tool_use]]\nmatcher = \"shell\"\ncommand = \"two\"\n\n\
         [[hooks.post_tool_use]]\ncommand = \"three\"\n\n\
         [[hooks.stop]]\ncommand = \"four\"\n",
    );
    let pre = resolved.hooks.for_event(HookEvent::PreToolUse);
    assert_eq!(pre.len(), 2);
    assert_eq!(
        pre[0].command().command_line(),
        "one",
        "declaration order is run order"
    );
    assert!(
        pre[0].watches("read"),
        "an absent matcher watches every tool"
    );
    assert!(pre[1].watches("shell"));
    assert!(!pre[1].watches("read"), "a named matcher is an exact name");
    assert_eq!(resolved.hooks.for_event(HookEvent::PostToolUse).len(), 1);
    assert_eq!(resolved.hooks.for_event(HookEvent::Stop).len(), 1);
    assert!(!resolved.hooks.is_empty());
}

/// The wildcard matcher watches every tool, the same as an absent one. It is
/// spelled out so a config can say "every tool" where the field is otherwise
/// a name.
#[test]
fn a_star_matcher_watches_every_tool() {
    let resolved = resolved("[[hooks.pre_tool_use]]\nmatcher = \"*\"\ncommand = \"true\"\n");
    let pre = resolved.hooks.for_event(HookEvent::PreToolUse);
    assert!(pre[0].watches("shell"));
    assert!(pre[0].watches("read"));
}

/// The argv form becomes one command line with every word quoted, so a word
/// carrying a space, a quote, or a `$` stays one word and expands to nothing.
#[test]
fn argv_is_quoted_into_one_command_line() {
    let resolved = resolved(
        "[[hooks.pre_tool_use]]\ncommand = [\"/bin/check\", \"a b\", \"it's\", \"$HOME\", \"\"]\n",
    );
    let pre = resolved.hooks.for_event(HookEvent::PreToolUse);
    assert_eq!(
        pre[0].command().command_line(),
        "'/bin/check' 'a b' 'it'\\''s' '$HOME' ''"
    );
}

#[test]
fn a_hook_without_a_timeout_gets_the_default() {
    let resolved = resolved("[[hooks.stop]]\ncommand = \"true\"\n");
    assert_eq!(
        resolved.hooks.for_event(HookEvent::Stop)[0].timeout(),
        std::time::Duration::from_millis(saya_config::DEFAULT_HOOK_TIMEOUT_MS)
    );
}

#[test]
fn a_timeout_over_the_ceiling_is_refused() {
    let toml = format!(
        "[[hooks.stop]]\ncommand = \"true\"\ntimeout_ms = {}\n",
        MAX_HOOK_TIMEOUT_MS + 1
    );
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_user(ConfigFile::from_toml(&toml).expect("fixture must parse"));
    let error = resolve(input).expect_err("an over-ceiling timeout must be refused");
    let message = error.to_string();
    assert!(
        message.contains("stop entry #1") && message.contains("ceiling"),
        "the refusal must name the entry and the bound: {message}"
    );
}

#[test]
fn a_zero_timeout_is_refused() {
    let input = ResolutionInput::new(ConnectionsFile::default()).with_user(
        ConfigFile::from_toml("[[hooks.stop]]\ncommand = \"true\"\ntimeout_ms = 0\n")
            .expect("fixture must parse"),
    );
    let error = resolve(input).expect_err("zero is a typo, not an instant pause");
    assert!(error.to_string().contains("at least one millisecond"));
}

#[test]
fn an_empty_command_is_refused() {
    let input = ResolutionInput::new(ConnectionsFile::default()).with_user(
        ConfigFile::from_toml("[[hooks.pre_tool_use]]\ncommand = \"   \"\n")
            .expect("fixture must parse"),
    );
    let error = resolve(input).expect_err("an empty command must be refused");
    assert!(error.to_string().contains("must not be empty"));
}

#[test]
fn an_empty_argv_is_refused() {
    let input = ResolutionInput::new(ConnectionsFile::default()).with_user(
        ConfigFile::from_toml("[[hooks.pre_tool_use]]\ncommand = []\n")
            .expect("fixture must parse"),
    );
    let error = resolve(input).expect_err("an empty argv must be refused");
    assert!(error.to_string().contains("at least one word"));
}

#[test]
fn an_oversized_command_is_refused() {
    let command = "x".repeat(saya_config::MAX_HOOK_COMMAND_BYTES + 1);
    let toml = format!("[[hooks.pre_tool_use]]\ncommand = \"{command}\"\n");
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_user(ConfigFile::from_toml(&toml).expect("fixture must parse"));
    let error = resolve(input).expect_err("an oversized command must be refused");
    assert!(error.to_string().contains("ceiling"));
}

#[test]
fn too_many_hooks_for_one_event_is_refused() {
    let mut toml = String::new();
    for _ in 0..=MAX_HOOKS_PER_EVENT {
        toml.push_str("[[hooks.stop]]\ncommand = \"true\"\n\n");
    }
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_user(ConfigFile::from_toml(&toml).expect("fixture must parse"));
    let error = resolve(input).expect_err("the per-event bound must hold");
    assert!(error.to_string().contains("hooks.stop lists"));
}

/// A config with no `[hooks]` table composes nothing: an unconfigured user
/// pays no process and no per-call cost.
#[test]
fn no_hooks_table_means_no_hooks() {
    let resolved = resolved("[ai]\nmodel = \"m\"\n");
    assert!(resolved.hooks.is_empty());
}

/// The three lifecycle events added for harness parity resolve in the same
/// table, carry their wire names, and are not empty.
#[test]
fn the_session_and_permission_events_resolve() {
    let resolved = resolved(
        "[[hooks.session_start]]\ncommand = \"start\"\n\n\
         [[hooks.session_end]]\ncommand = \"end\"\n\n\
         [[hooks.permission_request]]\nmatcher = \"shell\"\ncommand = \"check\"\n",
    );
    assert_eq!(resolved.hooks.for_event(HookEvent::SessionStart).len(), 1);
    assert_eq!(resolved.hooks.for_event(HookEvent::SessionEnd).len(), 1);
    let permission = resolved.hooks.for_event(HookEvent::PermissionRequest);
    assert_eq!(permission.len(), 1);
    assert!(permission[0].watches("shell"));
    assert!(!permission[0].watches("read"));
    assert_eq!(HookEvent::SessionStart.wire_name(), "SessionStart");
    assert_eq!(HookEvent::SessionEnd.wire_name(), "SessionEnd");
    assert_eq!(
        HookEvent::PermissionRequest.wire_name(),
        "PermissionRequest"
    );
    assert_eq!(HookEvent::ALL.len(), 6);
}
