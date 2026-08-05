use std::{collections::BTreeMap, fs};

use saya_types::SecretRef;

use crate::ConfigError;

/// Opaque resolved secret. It deliberately implements neither `Debug` nor `Serialize`.
pub struct ResolvedSecret(String);

impl ResolvedSecret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

pub trait SecretResolver: Send + Sync {
    fn resolve(&self, reference: &SecretRef) -> Result<ResolvedSecret, ConfigError>;
}

/// Runtime-independent resolver seam used by tests and later CLI environment wiring.
pub struct MapSecretResolver {
    values: BTreeMap<String, String>,
}

impl MapSecretResolver {
    pub fn new(values: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }
}

impl SecretResolver for MapSecretResolver {
    fn resolve(&self, reference: &SecretRef) -> Result<ResolvedSecret, ConfigError> {
        match reference {
            SecretRef::Env { env } => self
                .values
                .get(env)
                .cloned()
                .map(ResolvedSecret)
                .ok_or_else(|| ConfigError::MissingSecret(reference.redacted_label())),
            SecretRef::File { file } => fs::read_to_string(file)
                .map(|value| ResolvedSecret(value.trim_end().into()))
                .map_err(|_| ConfigError::SecretFile("[redacted path]".into())),
            SecretRef::Keyring { .. } => Err(ConfigError::KeyringUnavailable),
        }
    }
}
