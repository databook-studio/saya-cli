//! The pure configuration-reload core: build a candidate runtime from the
//! same inputs the launch loader consumed, classify every changed setting,
//! and re-seed only the session copies the user never overrode. Nothing here
//! touches a live session's authority-sensitive state, and a failed build
//! leaves the caller's runtime alone. O04 wires this and removes the gate.

pub(crate) mod classify;
pub(crate) mod plan;
pub(crate) mod report;

use crate::cli::GlobalOptions;
use crate::config::runtime::{RuntimeConfig, load_with_sources};
use crate::interactive::SessionState;
use plan::{ChangeLine, ReloadPlan};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// What a reload re-reads: the same inputs the startup loader consumed.
pub(crate) struct ReloadInputs {
    pub options: GlobalOptions,
    pub cwd: PathBuf,
    pub user_dir: PathBuf,
    pub process_env: BTreeMap<String, String>,
}

impl ReloadInputs {
    /// Production reads the CURRENT environment, so edits made during the
    /// session are picked up; tests pass explicit values instead.
    pub(crate) fn from_launch(options: &GlobalOptions) -> Self {
        Self {
            options: options.clone(),
            cwd: PathBuf::from("."),
            user_dir: super::sources::user_config_dir(),
            process_env: super::sources::process_env(),
        }
    }
}

/// A fully resolved candidate with no side effect. This is
/// `load_with_sources` — never `load`, which prints the ignored-override
/// warning; the plan reports those overrides itself.
pub(crate) fn build_candidate(inputs: &ReloadInputs) -> Result<RuntimeConfig, String> {
    load_with_sources(
        &inputs.options,
        &inputs.cwd,
        &inputs.user_dir,
        inputs.process_env.clone(),
    )
    .map_err(|error| error.to_string())
}

/// Classify every changed user-facing setting between the running runtime and
/// the candidate. Values come from the redacted diagnostics, so only
/// redacted labels can appear in the plan.
pub(crate) fn plan_reload(
    old: &RuntimeConfig,
    new: &RuntimeConfig,
    state: &SessionState,
) -> ReloadPlan {
    let mut plan = ReloadPlan {
        ignored_project_overrides: new.resolved.ignored_project_overrides.clone(),
        ..ReloadPlan::default()
    };
    let before = diagnostics_object(old);
    let after = diagnostics_object(new);
    let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    for key in keys {
        let (old_value, new_value) = (before.get(key), after.get(key));
        if old_value == new_value {
            continue;
        }
        classify::classify_change(
            &ChangeLine::new(
                key,
                old_value.unwrap_or(&Value::Null),
                new_value.unwrap_or(&Value::Null),
                None,
            ),
            old,
            new,
            state,
            &mut plan,
        );
    }
    plan.connections_change(old, new);
    plan.command_shaping_changes(old, new);
    for lines in [&mut plan.applied, &mut plan.reseeded, &mut plan.pending] {
        lines.sort_by(|a, b| a.key.cmp(&b.key));
    }
    plan
}

/// Perform ONLY the re-seeds on the session state. Data sharing, the approval
/// mode, grants, and the universe are never touched.
pub(crate) fn apply_plan(plan: &ReloadPlan, state: &mut SessionState) {
    if let Some(provider) = &plan.reseed.provider {
        state.provider = provider.clone();
    }
    if let Some(model) = &plan.reseed.model {
        state.model = model.clone();
    }
    if let Some(profile) = &plan.reseed.profile {
        state.profile = profile.clone();
    }
    if let Some(endpoint) = &plan.reseed.provider_endpoint {
        state.provider_endpoint = endpoint.clone();
        // Mirror `bind_runtime_endpoint`: once the reload writes the session's
        // endpoint, it is bound, so a later composition cannot silently
        // restore an old endpoint over the written value.
        state.provider_endpoint_bound = true;
    }
}

fn diagnostics_object(runtime: &RuntimeConfig) -> BTreeMap<String, Value> {
    match serde_json::to_value(runtime.resolved.redacted_diagnostics()) {
        Ok(Value::Object(map)) => map.into_iter().collect(),
        // A struct serialisation cannot fail here; an empty map degrades to
        // "no changes" rather than a wrong plan.
        _ => BTreeMap::new(),
    }
}

#[cfg(test)]
#[path = "reload_tests.rs"]
mod tests;
