//! `[hooks]` merging: one event's whole list at a time.

use crate::provenance::{Recorder, SettingSource};

use super::HooksFile;

/// A layer states an event's whole list or none of it. A hook list is a set of
/// commands, so a per-entry merge would silently keep a lower layer's command
/// alongside a higher layer's — two hooks where the user wrote one. Stating
/// the list replaces it, the way the `[tools]` allow list replaces its own.
pub(crate) fn merge(
    base: &mut HooksFile,
    layer: &HooksFile,
    recorder: &mut Recorder,
    source: impl Fn(&str) -> SettingSource,
) {
    for (name, incoming, target) in [
        ("pre_tool_use", &layer.pre_tool_use, &mut base.pre_tool_use),
        (
            "post_tool_use",
            &layer.post_tool_use,
            &mut base.post_tool_use,
        ),
        ("stop", &layer.stop, &mut base.stop),
        (
            "session_start",
            &layer.session_start,
            &mut base.session_start,
        ),
        ("session_end", &layer.session_end, &mut base.session_end),
        (
            "permission_request",
            &layer.permission_request,
            &mut base.permission_request,
        ),
    ] {
        if incoming.is_empty() {
            continue;
        }
        *target = incoming.clone();
        recorder.record(&format!("hooks.{name}"), source(name));
    }
}
