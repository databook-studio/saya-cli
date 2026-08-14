//! Mutating `preferences` commands: `set` and `unset`. Each resolves its scope
//! (and `set` its value), calls the `PreferenceStore`, and emits a confirmation
//! naming the kind and scope — never the value, which is untrusted input. A
//! write against an unavailable store exits non-zero, matching `contracts`.

use super::preferences_args::{PrefArgError, build_value, kind_str, required_scope};
use super::{EXIT_PREF_ERROR, STORE_UNAVAILABLE_MSG, resolve_scope};
use crate::cli::PreferenceKindArg;
use crate::commands::output::{failure_message, result};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_store::{PreferenceStore, SqliteStateStore};

pub(super) async fn set(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    kind: PreferenceKindArg,
    value: &str,
    profile: Option<String>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let value = match build_value(kind, value) {
        Ok(value) => value,
        Err(_) => return arg_failure(PrefArgError::InvalidValue, format),
    };
    let (scope, scope_name) = match resolve_scope(runtime, required_scope(kind), profile) {
        Ok(resolved) => resolved,
        Err((code, message)) => return failure_message(code, message, format),
    };
    if store.set_preference(&scope, value).await.is_err() {
        return failure_message(EXIT_PREF_ERROR, STORE_UNAVAILABLE_MSG.into(), format);
    }
    result(
        format!("set {} (scope: {})", kind_str(kind), scope_name),
        format,
    )
}

pub(super) async fn unset(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    kind: PreferenceKindArg,
    profile: Option<String>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let (scope, scope_name) = match resolve_scope(runtime, required_scope(kind), profile) {
        Ok(resolved) => resolved,
        Err((code, message)) => return failure_message(code, message, format),
    };
    if store
        .unset_preference(&scope, kind_str(kind))
        .await
        .is_err()
    {
        return failure_message(EXIT_PREF_ERROR, STORE_UNAVAILABLE_MSG.into(), format);
    }
    // A missing kind is a no-op in the store, so this always reads as success.
    result(
        format!("unset {} (scope: {})", kind_str(kind), scope_name),
        format,
    )
}

fn arg_failure(
    error: PrefArgError,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    failure_message(EXIT_PREF_ERROR, error.to_string(), format)
}
