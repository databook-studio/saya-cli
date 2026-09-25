use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
};

use saya_types::SecretRef;

use crate::ConfigError;

const SECRET_FILE_BYTE_LIMIT: usize = 1024 * 1024;

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
            SecretRef::File { file } => resolve_file_secret(file),
            SecretRef::Keyring { .. } => Err(ConfigError::KeyringUnavailable),
        }
    }
}

fn resolve_file_secret(path: &str) -> Result<ResolvedSecret, ConfigError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| ConfigError::SecretFile("[redacted path]".into()))?;
    if !metadata.file_type().is_file() {
        return Err(ConfigError::SecretFile("[redacted path]".into()));
    }

    let file = File::open(path).map_err(|_| ConfigError::SecretFile("[redacted path]".into()))?;
    if !file
        .metadata()
        .map_err(|_| ConfigError::SecretFile("[redacted path]".into()))?
        .file_type()
        .is_file()
    {
        return Err(ConfigError::SecretFile("[redacted path]".into()));
    }

    let mut bytes = Vec::new();
    file.take((SECRET_FILE_BYTE_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ConfigError::SecretFile("[redacted path]".into()))?;
    if bytes.len() > SECRET_FILE_BYTE_LIMIT {
        return Err(ConfigError::SecretFile("[redacted path]".into()));
    }
    let value =
        String::from_utf8(bytes).map_err(|_| ConfigError::SecretFile("[redacted path]".into()))?;
    Ok(ResolvedSecret(value.trim_end().into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "saya-config-secret-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn file_secret_is_trimmed() {
        let path = temp_path("trim");
        fs::write(&path, "secret-value\n\n").unwrap();
        let resolver = MapSecretResolver::new([]);
        let resolved = resolver
            .resolve(&SecretRef::File {
                file: path.to_string_lossy().into_owned(),
            })
            .unwrap();
        assert_eq!(resolved.expose(), "secret-value");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn oversized_file_secret_is_rejected_without_payload() {
        let path = temp_path("oversized");
        let sentinel = "secret-file-sentinel";
        let mut bytes = vec![b'x'; SECRET_FILE_BYTE_LIMIT + 1];
        bytes[..sentinel.len()].copy_from_slice(sentinel.as_bytes());
        fs::write(&path, bytes).unwrap();
        let resolver = MapSecretResolver::new([]);
        let error = match resolver.resolve(&SecretRef::File {
            file: path.to_string_lossy().into_owned(),
        }) {
            Ok(_) => panic!("oversized secret files must be rejected"),
            Err(error) => error,
        };
        let rendered = error.to_string();
        assert!(!rendered.contains(sentinel));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn nonregular_file_secret_is_rejected() {
        let path = temp_path("directory");
        fs::create_dir(&path).unwrap();
        let resolver = MapSecretResolver::new([]);
        let error = match resolver.resolve(&SecretRef::File {
            file: path.to_string_lossy().into_owned(),
        }) {
            Ok(_) => panic!("directories are not secret files"),
            Err(error) => error,
        };
        assert!(matches!(error, ConfigError::SecretFile(_)));
        let _ = fs::remove_dir(path);
    }
}
