//! The one-line announcement of an automatically chosen provider, and the
//! terminal prompt that resolves two or more provider keys.
//!
//! Both live beside the loader because they are the only two places the
//! environment's provider keys become a visible decision. The announcement is
//! derived from provenance, not from a second look at the environment, so a
//! provider stated by a layer can never be announced as automatic.

use std::{collections::BTreeMap, io::IsTerminal};

use saya_config::{ProviderKey, SettingSource, provider_keys_in_env};

use super::runtime::RuntimeConfig;

/// Says which provider and model the environment's one key chose, and how to
/// change them. One line, on stderr, so it is visible beside a headless
/// answer and does not enter the answer itself.
pub(crate) fn announce(runtime: &RuntimeConfig) {
    let Some(origin) = runtime.resolved.provenance.get("ai.provider") else {
        return;
    };
    let SettingSource::AutoEnv { var } = &origin.source else {
        return;
    };
    eprintln!(
        "Using {} with {} from {var}. Change it with --provider or [ai] in config.toml.",
        runtime.resolved.ai.provider.as_str(),
        runtime.resolved.ai.model,
    );
}

/// Asks which of the present provider keys to use, when there is a terminal to
/// ask on. Returns `None` when there is no terminal, fewer than two keys, or
/// no usable answer — the caller then reports the ambiguity rather than
/// guessing.
pub(crate) fn prompt_for_provider(
    process: &BTreeMap<String, String>,
) -> Option<&'static ProviderKey> {
    if !std::io::stdin().is_terminal() {
        return None;
    }
    let choices = provider_keys_in_env(process);
    if choices.len() < 2 {
        return None;
    }
    eprintln!("more than one AI provider key is set:");
    for (index, key) in choices.iter().enumerate() {
        eprintln!("  {}. {} ({})", index + 1, key.provider.as_str(), key.env);
    }
    eprint!("choose a provider [1-{}]: ", choices.len());
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).ok()? == 0 {
        return None;
    }
    let choice: usize = line.trim().parse().ok()?;
    choices.get(choice.checked_sub(1)?).copied()
}
