use crate::{SettingSource, ToolsFile, provenance::Recorder};

/// Merges one layer's `[tools]` table over the base. A stated `allow` replaces
/// the lower layer's whole list — the setting is one set of names, not a set
/// that accumulates across layers, so a user config cannot widen a launch
/// flag's narrowing by naming more tools.
pub(crate) fn merge(
    base: &mut ToolsFile,
    layer: &ToolsFile,
    recorder: &mut Recorder,
    source: impl Fn(&str) -> SettingSource,
) {
    if layer.allow.is_some() {
        base.allow = layer.allow.clone();
        recorder.record("tools.allow", source("allow"));
    }
}
