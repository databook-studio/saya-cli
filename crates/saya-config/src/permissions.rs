//! The `[permissions]` setting: whether a session may enter `bypass`, and the
//! command prefixes a call may run without asking.
//!
//! `allow_bypass` is a gate rather than a mode: it does not put any session
//! into `bypass`, it decides whether the mode is *reachable* — by `/mode`, by
//! the Shift+Tab cycle, or by the launch's `--permission-mode`. With the key
//! unset (the default) the mode is refused wherever it is asked for, and a
//! resumed session whose record says `bypass` degrades to `ask`.
//!
//! ```toml
//! [permissions]
//! allow_bypass = true
//! allow = ["cargo test", "git diff"]
//! deny = ["rm"]
//! read_only_commands = false     # default: true
//! ```
//!
//! `allow` and `deny` are command *prefixes*, matched token by token by
//! [`rules::decide`]. A deny rule refuses the call in every mode, including
//! `bypass`; an allow rule lets every segment of a compound command take the
//! host lane without asking under `ask`. Neither widens `read-only` or
//! `never`, and neither auto-allows a command that contains a substitution or
//! heredoc.
//!
//! `read_only_commands` is the switch over the built-in vetted read-only set:
//! with it on (the default) a command whose every part only reads runs under
//! `ask` and `accept-edits` without a question, and with it off every command
//! is put to the operator as it was before the set existed. It can withhold
//! that set; it cannot state one.
//!
//! The table is protected whole (see [`crate::layers`]): a repository's
//! `.saya/config.toml` is untrusted input, and a model-writable file must
//! not be able to hand itself the one consent that answers every per-call
//! question — nor state the rules that pre-answer a command. The user layer
//! is the only layer that can state them, which is what makes the refusal
//! message's instruction true.

mod merge;
pub mod rules;

pub(crate) use merge::merge;
pub use rules::{
    CommandDecision, CommandRule, CommandRules, MAX_COMMAND_RULE_BYTES, MAX_COMMAND_RULES,
    ReadOnlySet, decide,
};

use serde::Deserialize;

/// Raw trusted `[permissions]` settings. Every field stays optional until the
/// layers merge, so a higher-precedence layer can override one setting at a
/// time and a table that states nothing resolves to the closed direction.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PermissionsFile {
    /// Whether a session may enter `bypass`. `None` — the default — withholds
    /// it. A session's `--approval-mode bypass` is that process's own explicit
    /// consent and is unaffected: this key governs the session's mode switch,
    /// not a launch that already stated the posture itself.
    pub allow_bypass: Option<bool>,
    /// Command prefixes that run on the host without asking under `ask`.
    pub allow: Option<Vec<String>>,
    /// Command prefixes that are refused in every mode, `bypass` included.
    pub deny: Option<Vec<String>>,
    /// Whether the built-in vetted read-only set may skip the question under
    /// `ask` and `accept-edits`. `None` — the default — offers it; `false` puts
    /// every command to the operator.
    pub read_only_commands: Option<bool>,
}

/// The resolved `[permissions]` table. `allow_bypass: false` — the default —
/// is what "the configuration does not allow bypass" means everywhere
/// downstream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedPermissions {
    pub allow_bypass: bool,
    /// The trusted rules, with every refused entry recorded as a diagnostic
    /// rather than a panic.
    pub rules: CommandRules,
}

pub(crate) fn resolve(file: &PermissionsFile) -> ResolvedPermissions {
    let (allow, mut diagnostics) = resolve_list(file.allow.as_deref(), "allow");
    let (deny, deny_diagnostics) = resolve_list(file.deny.as_deref(), "deny");
    diagnostics.extend(deny_diagnostics);
    ResolvedPermissions {
        allow_bypass: file.allow_bypass.unwrap_or(false),
        rules: CommandRules {
            allow,
            deny,
            diagnostics,
            read_only_set: resolve_read_only_set(file.read_only_commands),
        },
    }
}

/// The vetted read-only set is offered unless the operator withheld it: an
/// unset key is the default, and only `false` turns it off.
fn resolve_read_only_set(declared: Option<bool>) -> ReadOnlySet {
    if declared == Some(false) {
        ReadOnlySet::Off
    } else {
        ReadOnlySet::On
    }
}

fn resolve_list(raw: Option<&[String]>, field: &str) -> (Vec<CommandRule>, Vec<String>) {
    let mut rules = Vec::new();
    let mut diagnostics = Vec::new();
    for (index, text) in raw.unwrap_or_default().iter().enumerate() {
        if index >= MAX_COMMAND_RULES {
            diagnostics.push(format!(
                "permissions.{field}: more than {MAX_COMMAND_RULES} rules; the rest are ignored"
            ));
            break;
        }
        match CommandRule::parse(text) {
            Ok(rule) => rules.push(rule),
            Err(error) => diagnostics.push(format!("permissions.{field}[{index}]: {error}")),
        }
    }
    (rules, diagnostics)
}
