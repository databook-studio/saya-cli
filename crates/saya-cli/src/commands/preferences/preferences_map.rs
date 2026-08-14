//! Mapping typed `PreferenceValue`s to render-owned DTOs. Presentation only:
//! turns a `(kind, value)` pair (which carries no identity) into a
//! [`PreferenceView`] tagged with the scope *name* the caller resolved. The
//! opaque `ProfileIdentity` is dropped before this call and has no field on the
//! DTO — mirroring `contracts_map.rs`.

use crate::render::PreferenceView;
use saya_types::{DateGrain, OutputStyle, PreferenceValue};

/// The short rendered form of a value: the timezone string, the grain, the
/// style, or the profile name. Matches the kind the DTO carries.
pub(super) fn render_value(value: &PreferenceValue) -> String {
    match value {
        PreferenceValue::Timezone { value, .. } => value.clone(),
        PreferenceValue::DateGrain { grain, .. } => grain_str(*grain).into(),
        PreferenceValue::OutputStyle { style, .. } => style_str(*style).into(),
        PreferenceValue::DefaultProfile { name, .. } => name.clone(),
        // `PreferenceValue` is `#[non_exhaustive]`; a future variant renders its
        // kind with no value, leaking nothing the type does not already expose.
        _ => String::new(),
    }
}

/// Maps one stored `(kind, value)` pair to a render DTO at `scope_name`. The
/// scope name is the profile *name* or `"global"` — never the identity.
pub(super) fn preference_view(
    (kind, value): &(String, PreferenceValue),
    scope_name: &str,
) -> PreferenceView {
    PreferenceView {
        kind: kind.clone(),
        value: render_value(value),
        scope: scope_name.to_string(),
    }
}

fn grain_str(grain: DateGrain) -> &'static str {
    match grain {
        DateGrain::Day => "day",
        DateGrain::Week => "week",
        DateGrain::Month => "month",
        DateGrain::Quarter => "quarter",
        DateGrain::Year => "year",
        // `DateGrain` is `#[non_exhaustive]`; a future grain renders generically.
        _ => "unknown",
    }
}

fn style_str(style: OutputStyle) -> &'static str {
    match style {
        OutputStyle::Table => "table",
        OutputStyle::Compact => "compact",
        OutputStyle::Narrative => "narrative",
        // `OutputStyle` is `#[non_exhaustive]`; a future style renders generically.
        _ => "unknown",
    }
}
