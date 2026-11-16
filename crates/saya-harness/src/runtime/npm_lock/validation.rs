//! Exact container-mount and lock source validation rules.

use std::path::Path;

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
};
use sha2::{Digest as _, Sha256};
use url::Url;

use super::super::{NODE_IMAGE_MANIFEST_DIGEST, NODE_VERSION, NPM_VERSION, npm::NpmRuntimeError};

pub(super) const INPUT_POLICY_VERSION: &str =
    "registry-npmjs-lock-v3-sha512-offline-controller-v2-max128mib";
const MAX_RESOLVED_URL_BYTES: usize = 4 * 1024;
const MAX_INTEGRITY_BYTES: usize = 4 * 1024;
const NPM_REGISTRY_HOST: &str = "registry.npmjs.org";

pub(super) fn input_key(manifest_sha256: &str, lock_sha256: &str) -> String {
    let mut digest = Sha256::new();
    let controller_sha256 = Sha256::digest(include_bytes!("../npm_controller.js"));
    for part in [
        "saya-npm-input-v1",
        manifest_sha256,
        lock_sha256,
        NODE_IMAGE_MANIFEST_DIGEST,
        NODE_VERSION,
        NPM_VERSION,
        INPUT_POLICY_VERSION,
    ] {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    digest.update(64u64.to_be_bytes());
    digest.update(hex_digest(&controller_sha256).as_bytes());
    hex_digest(&digest.finalize())
}

pub(in crate::runtime) fn effective_toolchain_id(
    input_key_sha256: &str,
    bundle_sha256: &str,
) -> String {
    let mut digest = Sha256::new();
    for part in [
        "saya-node-toolchain-v2",
        input_key_sha256,
        bundle_sha256,
        super::super::npm_snapshot::SOURCE_SNAPSHOT_POLICY_ID,
    ] {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    format!("saya-node-v2-{}", hex_digest(&digest.finalize()))
}

pub(in crate::runtime) fn validate_mount_path(path: &Path) -> Result<(), NpmRuntimeError> {
    let Some(path) = path.to_str() else {
        return Err(NpmRuntimeError::InvalidProject);
    };
    if path
        .bytes()
        .any(|byte| byte == b':' || byte == b',' || byte.is_ascii_control())
    {
        return Err(NpmRuntimeError::InvalidProject);
    }
    Ok(())
}

pub(super) fn validate_dependency_specifier(value: &str) -> Result<(), NpmRuntimeError> {
    if value.is_empty()
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b'\\')
        || value.contains(':')
        || value.contains('/')
    {
        return Err(NpmRuntimeError::InvalidLock);
    }
    Ok(())
}

pub(super) fn validate_package_path(path: &str) -> Result<(), NpmRuntimeError> {
    if !path.starts_with("node_modules/")
        || path.contains('\\')
        || path.bytes().any(|byte| byte.is_ascii_control())
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(NpmRuntimeError::InvalidLock);
    }
    Ok(())
}

pub(super) fn validate_registry_url(value: &str) -> Result<String, NpmRuntimeError> {
    if value.len() > MAX_RESOLVED_URL_BYTES || value.contains('#') || value.contains('?') {
        return Err(NpmRuntimeError::UntrustedOrigin);
    }
    let authority = value
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .ok_or(NpmRuntimeError::UntrustedOrigin)?;
    if authority.contains('@') {
        return Err(NpmRuntimeError::UntrustedOrigin);
    }
    let url = Url::parse(value).map_err(|_| NpmRuntimeError::UntrustedOrigin)?;
    if url.scheme() != "https"
        || url.host_str() != Some(NPM_REGISTRY_HOST)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port().is_some_and(|port| port != 443)
        || url.path() == "/"
    {
        return Err(NpmRuntimeError::UntrustedOrigin);
    }
    Ok(url.to_string())
}

pub(super) fn parse_sha512_integrities(value: &str) -> Result<Vec<[u8; 64]>, NpmRuntimeError> {
    if value.len() > MAX_INTEGRITY_BYTES {
        return Err(NpmRuntimeError::InvalidIntegrity);
    }
    let mut digests = Vec::new();
    for item in value.split_ascii_whitespace() {
        let Some((algorithm, encoded)) = item.split_once('-') else {
            continue;
        };
        if algorithm != "sha512" {
            continue;
        }
        let decoded = STANDARD
            .decode(encoded)
            .or_else(|_| STANDARD_NO_PAD.decode(encoded))
            .map_err(|_| NpmRuntimeError::InvalidIntegrity)?;
        let digest: [u8; 64] = decoded
            .try_into()
            .map_err(|_| NpmRuntimeError::InvalidIntegrity)?;
        digests.push(digest);
    }
    if digests.is_empty() {
        return Err(NpmRuntimeError::InvalidIntegrity);
    }
    Ok(digests)
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}
