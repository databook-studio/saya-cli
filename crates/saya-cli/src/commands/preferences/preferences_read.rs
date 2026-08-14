//! Read-only `preferences list`. Resolves the scope, reads global + the
//! resolved profile's preferences, merges them (the two scopes hold disjoint
//! kinds), sorts by kind, and emits a `PreferenceList` event. An unreadable
//! store is a diagnostic on this path, not a crash — matching `contracts list`.

use super::preferences_map::preference_view;
use super::{EXIT_PREF_ERROR, STORE_UNAVAILABLE_MSG, resolve_profile};
use crate::commands::output::{emit, failure_message};
use crate::config::runtime::RuntimeConfig;
use crate::render::{RenderFormat, TerminalEvent};
use saya_store::{PreferenceStore, SqliteStateStore};
use saya_types::PreferenceScope;

pub(super) async fn list(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    profile: Option<String>,
) -> Result<i32, Box<dyn std::error::Error>> {
    // Global preferences are always visible; the profile half uses the resolved
    // profile (active by default, or `--profile`). The two scopes hold disjoint
    // kinds, so a merged list loses no row and a per-kind `list` would hide the
    // global kinds (there is no `--global` flag). Sorted by kind for determinism.
    let global = match store.list_preferences(&PreferenceScope::Global).await {
        Ok(rows) => rows,
        Err(_) => return failure_message(EXIT_PREF_ERROR, STORE_UNAVAILABLE_MSG.into(), format),
    };
    let (name, identity) = match resolve_profile(runtime, profile.as_deref()) {
        Ok(value) => value,
        Err((code, message)) => return failure_message(code, message, format),
    };
    let scoped = match store
        .list_preferences(&PreferenceScope::Profile(identity))
        .await
    {
        Ok(rows) => rows,
        Err(_) => return failure_message(EXIT_PREF_ERROR, STORE_UNAVAILABLE_MSG.into(), format),
    };
    let mut views: Vec<_> = global
        .iter()
        .map(|row| preference_view(row, "global"))
        .collect();
    views.extend(scoped.iter().map(|row| preference_view(row, &name)));
    views.sort_by(|a, b| a.kind.cmp(&b.kind));
    emit(TerminalEvent::PreferenceList { preferences: views }, format);
    Ok(0)
}
