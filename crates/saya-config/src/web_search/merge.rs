use crate::{SettingSource, WebSearchFile, provenance::Recorder};

/// Merges one layer's `[web_search]` table over the base, field by field: a
/// stated setting replaces the lower layer's, and an unstated one is left as
/// it was, so a launch-time override can retarget one field without restating
/// the table.
///
/// The credential records the winning reference's redacted label rather than
/// the layer alone — the reference is the only display-safe fact about a
/// secret, the same rule `ai.api_key` follows.
pub(crate) fn merge(
    base: &mut WebSearchFile,
    layer: &WebSearchFile,
    recorder: &mut Recorder,
    source: impl Fn(&str) -> SettingSource,
) {
    if layer.provider.is_some() {
        base.provider = layer.provider.clone();
        recorder.record("web_search.provider", source("provider"));
    }
    if layer.endpoint.is_some() {
        base.endpoint = layer.endpoint.clone();
        recorder.record("web_search.endpoint", source("endpoint"));
    }
    if let Some(secret) = layer.api_key.as_ref() {
        base.api_key = Some(secret.clone());
        recorder.record_secret("web_search.api_key", source("api_key"), secret);
    }
}
