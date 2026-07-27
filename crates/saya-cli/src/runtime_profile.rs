use crate::config_runtime::{RuntimeConfig, RuntimeError};

impl RuntimeConfig {
    pub fn secret_resolver(&self) -> saya_config::MapSecretResolver {
        saya_config::MapSecretResolver::new(self.secret_values.clone())
    }

    pub fn named_profile(&self, name: &str) -> Result<&saya_types::DatabaseProfile, RuntimeError> {
        if self.resolved.profile_name.as_deref() == Some(name) {
            if let Some(profile) = self.resolved.profile.as_ref() {
                return Ok(profile);
            }
        }
        self.connections.profiles.get(name).ok_or_else(|| {
            RuntimeError::Config(saya_config::ConfigError::UnknownProfile(name.into()))
        })
    }
}
