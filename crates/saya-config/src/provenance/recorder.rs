//! The write side: origins are recorded where a merge funnel assigns, and
//! can be rolled back to a pre-project snapshot by the trust boundary.

use saya_types::SecretRef;

use super::source::{SettingOrigin, SettingProvenance, SettingSource};
use crate::model::EndpointFile;

/// The write side: called from `layers.rs` where a layer assigns, so the
/// recording cannot drift from the merge it observes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Recorder {
    origins: SettingProvenance,
}

impl Recorder {
    pub(crate) fn record(&mut self, key: &str, source: SettingSource) {
        self.record_reference(key, source, None);
    }

    /// Records a secret-bearing key with the reference's redacted label.
    pub(crate) fn record_secret(&mut self, key: &str, source: SettingSource, secret: &SecretRef) {
        self.record_reference(key, source, Some(secret.redacted_label()));
    }

    pub(crate) fn record_reference(
        &mut self,
        key: &str,
        source: SettingSource,
        reference: Option<String>,
    ) {
        self.origins
            .0
            .insert(key.to_owned(), SettingOrigin { source, reference });
    }

    /// The pre-project copy `rollback` restores from.
    pub(crate) fn snapshot(&self) -> SettingProvenance {
        self.origins.clone()
    }

    /// Rolls the keys `revert_untrusted` reported back to the pre-project
    /// snapshot: restored where the snapshot holds the entry, removed where
    /// it does not (a project-only endpoint). A whole-entry endpoint key
    /// (`ai.endpoints["name"]`) rolls its recorded per-field descendants.
    pub(crate) fn rollback(&mut self, keys: &[String], snapshot: &SettingProvenance) {
        for key in keys {
            self.restore(key, snapshot);
            let prefix = format!("{key}.");
            let descendants: Vec<String> = self
                .origins
                .0
                .range(key.clone()..)
                .take_while(|(candidate, _)| candidate.starts_with(&prefix))
                .map(|(candidate, _)| candidate.clone())
                .collect();
            for candidate in descendants {
                self.restore(&candidate, snapshot);
            }
        }
    }

    fn restore(&mut self, key: &str, snapshot: &SettingProvenance) {
        match snapshot.0.get(key) {
            Some(origin) => self.origins.0.insert(key.to_owned(), origin.clone()),
            None => self.origins.0.remove(key),
        };
    }

    pub(crate) fn origin_of(&self, key: &str) -> Option<&SettingOrigin> {
        self.origins.0.get(key)
    }

    pub(crate) fn finish(self) -> SettingProvenance {
        self.origins
    }
}

/// Records the `[[ai.endpoints]]` fields a layer supplied: an overlay
/// records exactly the fields it set; a new entry, the fields it declares.
/// Keys use the `ai.endpoints["name"].field` shape `revert_untrusted`
/// reports, so its ignored list alone drives the rollback.
pub(crate) fn record_endpoint_fields(
    recorder: &mut Recorder,
    name: &str,
    fields: &EndpointFile,
    source: SettingSource,
) {
    let base = format!("ai.endpoints[{name:?}]");
    for (field, supplied) in [
        ("provider", fields.provider.is_some()),
        ("model", fields.model.is_some()),
        ("base_url", fields.base_url.is_some()),
    ] {
        if supplied {
            recorder.record(&format!("{base}.{field}"), source.clone());
        }
    }
    if let Some(secret) = fields.api_key.as_ref() {
        recorder.record_secret(&format!("{base}.api_key"), source.clone(), secret);
    }
}
