use crate::{AgentsFile, SettingSource, provenance::Recorder};

/// Merges one `[agents]` layer over the base: each setting it states, and — as
/// one table — the types it declares. The types replace rather than merge entry
/// by entry, because an entry already states only what it changes about the
/// type it names: merging twice would let a higher layer's silence about a
/// field be read as a statement about it.
pub(crate) fn merge(
    base: &mut AgentsFile,
    layer: &AgentsFile,
    recorder: &mut Recorder,
    source: impl Fn(&str) -> SettingSource,
) {
    if layer.max_children.is_some() {
        base.max_children = layer.max_children;
        recorder.record("agents.max_children", source("max_children"));
    }
    if layer.max_concurrent_model_requests.is_some() {
        base.max_concurrent_model_requests = layer.max_concurrent_model_requests;
        recorder.record(
            "agents.max_concurrent_model_requests",
            source("max_concurrent_model_requests"),
        );
    }
    if !layer.types.is_empty() {
        base.types = layer.types.clone();
        recorder.record("agents", source("types"));
    }
}
