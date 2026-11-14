//! Provenance is value-free: a secret appears only as its redacted
//! reference label — never the value, never the file path — even in the
//! serialized map.

use saya_config::{ConfigFile, SettingSource, resolve};

use super::empty;

#[test]
fn provenance_never_carries_a_secret_value() {
    let resolved =
        resolve(empty().with_process_env([("SAYA_AI_API_KEY", "sentinel-secret-value")])).unwrap();
    let origin = resolved.provenance.get("ai.api_key").unwrap();
    assert_eq!(
        origin.source,
        SettingSource::Env {
            var: "SAYA_AI_API_KEY".into()
        }
    );
    assert_eq!(origin.reference.as_deref(), Some("env:SAYA_AI_API_KEY"));
    let json = serde_json::to_string(&resolved.provenance).unwrap();
    assert!(!json.contains("sentinel-secret-value"), "{json}");
}

#[test]
fn provenance_never_carries_a_secret_path() {
    let resolved = resolve(empty().with_user(
        ConfigFile::from_toml("[ai]\napi_key = { file = '/home/secret-user/key.txt' }\n").unwrap(),
    ))
    .unwrap();
    let origin = resolved.provenance.get("ai.api_key").unwrap();
    assert_eq!(origin.source, SettingSource::UserFile);
    assert_eq!(origin.reference.as_deref(), Some("file:[redacted path]"));
    let json = serde_json::to_string(&resolved.provenance).unwrap();
    assert!(!json.contains("secret-user"), "{json}");
    assert!(!json.contains("key.txt"), "{json}");
}
