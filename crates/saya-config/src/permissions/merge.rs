use crate::{PermissionsFile, SettingSource, provenance::Recorder};

/// Merges one layer's `[permissions]` table over the base, field by field: a
/// stated key replaces the lower layer's, and an unstated one is left as it
/// was. A list is one statement, so a layer states it whole or not at all —
/// flag rules are added separately by the CLI layer.
pub(crate) fn merge(
    base: &mut PermissionsFile,
    layer: &PermissionsFile,
    recorder: &mut Recorder,
    source: impl Fn(&str) -> SettingSource,
) {
    if layer.allow_bypass.is_some() {
        base.allow_bypass = layer.allow_bypass;
        recorder.record("permissions.allow_bypass", source("allow_bypass"));
    }
    if layer.allow.is_some() {
        base.allow = layer.allow.clone();
        recorder.record("permissions.allow", source("allow"));
    }
    if layer.deny.is_some() {
        base.deny = layer.deny.clone();
        recorder.record("permissions.deny", source("deny"));
    }
}
