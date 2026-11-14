//! Bounded package.json/package-lock binding and lock-v3 validation.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::{
    HarnessError,
    workspace::{ReadFile, Workspace},
};

use super::npm::NpmRuntimeError;

#[path = "npm_lock/validation.rs"]
mod validation;
pub(super) use validation::{effective_toolchain_id, validate_mount_path};

pub(super) const MAX_PACKAGE_MANIFEST_BYTES: u64 = 256 * 1024;
pub(super) const MAX_PACKAGE_LOCK_BYTES: u64 = 4 * 1024 * 1024;
pub(super) const MAX_NPMRC_BYTES: u64 = 8 * 1024;
pub(super) const MAX_NPM_PACKAGES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NpmInputBinding {
    manifest_sha256: String,
    lock_sha256: String,
    input_key_sha256: String,
    manifest_bytes: Vec<u8>,
    lock_bytes: Vec<u8>,
    tarballs: Vec<LockedNpmPackage>,
}

impl NpmInputBinding {
    pub(super) fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    pub(super) fn lock_sha256(&self) -> &str {
        &self.lock_sha256
    }

    pub(super) fn input_key_sha256(&self) -> &str {
        &self.input_key_sha256
    }

    pub(super) fn manifest_bytes(&self) -> &[u8] {
        &self.manifest_bytes
    }

    pub(super) fn lock_bytes(&self) -> &[u8] {
        &self.lock_bytes
    }

    pub(super) fn tarballs(&self) -> &[LockedNpmPackage] {
        &self.tarballs
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LockedNpmPackage {
    package_path: String,
    resolved_url: String,
    sha512_integrities: Vec<[u8; 64]>,
}

impl LockedNpmPackage {
    pub(super) fn package_path(&self) -> &str {
        &self.package_path
    }

    pub(super) fn resolved_url(&self) -> &str {
        &self.resolved_url
    }

    pub(super) fn sha512_integrities(&self) -> &[[u8; 64]] {
        &self.sha512_integrities
    }
}

pub(super) fn bind_project_inputs(project: &Workspace) -> Result<NpmInputBinding, NpmRuntimeError> {
    validate_mount_path(project.root())?;
    let manifest = read_required(
        project,
        "package.json",
        MAX_PACKAGE_MANIFEST_BYTES,
        NpmRuntimeError::InvalidManifest,
        NpmRuntimeError::InvalidManifest,
    )?;
    let lock = read_required(
        project,
        "package-lock.json",
        MAX_PACKAGE_LOCK_BYTES,
        NpmRuntimeError::InvalidLock,
        NpmRuntimeError::LockTooLarge,
    )?;
    match project.read(".npmrc", MAX_NPMRC_BYTES) {
        Ok(_) => return Err(NpmRuntimeError::NpmrcRefused),
        Err(error) if error.is_not_found() => {}
        Err(_) => return Err(NpmRuntimeError::NpmrcRefused),
    }
    let manifest_value: Value =
        serde_json::from_slice(&manifest.bytes).map_err(|_| NpmRuntimeError::InvalidManifest)?;
    let lock_value: Value =
        serde_json::from_slice(&lock.bytes).map_err(|_| NpmRuntimeError::InvalidLock)?;
    validate_lock_shape(&manifest_value, &lock_value)?;
    let manifest_sha256 = manifest.digest;
    let lock_sha256 = lock.digest;
    let manifest_bytes = manifest.bytes;
    let lock_bytes = lock.bytes;
    let tarballs = parse_tarballs(&lock_value)?;
    let input_key_sha256 = validation::input_key(&manifest_sha256, &lock_sha256);
    Ok(NpmInputBinding {
        manifest_sha256,
        lock_sha256,
        input_key_sha256,
        manifest_bytes,
        lock_bytes,
        tarballs,
    })
}

fn read_required(
    project: &Workspace,
    path: &str,
    limit: u64,
    missing: NpmRuntimeError,
    too_large: NpmRuntimeError,
) -> Result<ReadFile, NpmRuntimeError> {
    match project.read(path, limit) {
        Ok(file) if file.size <= limit && !file.truncated => Ok(file),
        Ok(_) | Err(HarnessError::BoundsExceeded { .. }) => Err(too_large),
        Err(error) if error.is_not_found() => Err(missing),
        Err(_) => Err(missing),
    }
}

fn validate_lock_shape(manifest: &Value, lock: &Value) -> Result<(), NpmRuntimeError> {
    let manifest = manifest
        .as_object()
        .ok_or(NpmRuntimeError::InvalidManifest)?;
    let lock = lock.as_object().ok_or(NpmRuntimeError::InvalidLock)?;
    if manifest.contains_key("workspaces") {
        return Err(NpmRuntimeError::InvalidLock);
    }
    if lock.get("lockfileVersion").and_then(Value::as_u64) != Some(3) {
        return Err(NpmRuntimeError::UnsupportedLock);
    }
    let packages = lock
        .get("packages")
        .and_then(Value::as_object)
        .ok_or(NpmRuntimeError::InvalidLock)?;
    if packages.is_empty() || packages.len().saturating_sub(1) > MAX_NPM_PACKAGES {
        return Err(NpmRuntimeError::TooManyPackages);
    }
    let root = packages
        .get("")
        .and_then(Value::as_object)
        .ok_or(NpmRuntimeError::InvalidLock)?;
    for field in ["name", "version"] {
        if let Some(name) = manifest.get(field)
            && root.get(field) != Some(name)
        {
            return Err(NpmRuntimeError::ManifestLockMismatch);
        }
    }
    for field in [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ] {
        let manifest_deps = dependency_map(manifest.get(field), NpmRuntimeError::InvalidManifest)?;
        let lock_deps = dependency_map(root.get(field), NpmRuntimeError::InvalidLock)?;
        if manifest_deps != lock_deps {
            return Err(NpmRuntimeError::ManifestLockMismatch);
        }
        for specifier in manifest_deps.values() {
            validation::validate_dependency_specifier(specifier)
                .map_err(|_| NpmRuntimeError::InvalidManifest)?;
        }
    }
    Ok(())
}

fn dependency_map(
    value: Option<&Value>,
    invalid: NpmRuntimeError,
) -> Result<BTreeMap<String, String>, NpmRuntimeError> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    value
        .as_object()
        .ok_or(invalid)?
        .iter()
        .map(|(name, specifier)| {
            let specifier = specifier.as_str().ok_or(invalid)?.to_owned();
            validation::validate_dependency_specifier(&specifier).map_err(|_| invalid)?;
            Ok((name.clone(), specifier))
        })
        .collect()
}

fn parse_tarballs(lock: &Value) -> Result<Vec<LockedNpmPackage>, NpmRuntimeError> {
    let packages = lock
        .get("packages")
        .and_then(Value::as_object)
        .ok_or(NpmRuntimeError::InvalidLock)?;
    let mut tarballs = Vec::with_capacity(packages.len().saturating_sub(1));
    for (package_path, package) in packages {
        if package_path.is_empty() {
            continue;
        }
        validation::validate_package_path(package_path)?;
        let metadata = package.as_object().ok_or(NpmRuntimeError::InvalidLock)?;
        if metadata.get("link").and_then(Value::as_bool) == Some(true) {
            return Err(NpmRuntimeError::InvalidLock);
        }
        for field in [
            "dependencies",
            "devDependencies",
            "optionalDependencies",
            "peerDependencies",
        ] {
            for specifier in
                dependency_map(metadata.get(field), NpmRuntimeError::InvalidLock)?.values()
            {
                validation::validate_dependency_specifier(specifier)?;
            }
        }
        let resolved = metadata
            .get("resolved")
            .and_then(Value::as_str)
            .ok_or(NpmRuntimeError::InvalidLock)?;
        let resolved_url = validation::validate_registry_url(resolved)?;
        let integrity = metadata
            .get("integrity")
            .and_then(Value::as_str)
            .ok_or(NpmRuntimeError::InvalidIntegrity)?;
        let sha512_integrities = validation::parse_sha512_integrities(integrity)?;
        tarballs.push(LockedNpmPackage {
            package_path: package_path.clone(),
            resolved_url,
            sha512_integrities,
        });
    }
    Ok(tarballs)
}
