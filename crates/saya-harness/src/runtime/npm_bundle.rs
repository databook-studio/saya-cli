//! Bounded host verification and publication of lock-authorized tarball inputs.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    npm::NpmRuntimeError,
    npm_lock::NpmInputBinding,
    registry::{Registry, RegistryError},
};

mod io;
mod verify;

pub(super) const MAX_TARBALL_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const MAX_BUNDLE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(super) struct VerifiedBundle {
    pub(super) path: PathBuf,
    pub(super) bundle_sha256: String,
    pub(super) tarball_count: usize,
    pub(super) tarball_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct BundleManifest {
    version: u8,
    input_key_sha256: String,
    manifest_sha256: String,
    lock_sha256: String,
    tarballs: Vec<BundleTarball>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BundleTarball {
    package_path: String,
    resolved_url: String,
    file: String,
    bytes: u64,
    sha512: String,
}

pub(super) fn create_stage_files(
    stage: &Path,
    binding: &NpmInputBinding,
) -> Result<(), NpmRuntimeError> {
    io::write_private_file(&stage.join("package.json"), binding.manifest_bytes())?;
    io::write_private_file(&stage.join("package-lock.json"), binding.lock_bytes())?;
    io::create_private_dir(&stage.join("tarballs"))?;
    io::write_private_file(
        &stage.join("npm-controller.js"),
        include_bytes!("npm_controller.js"),
    )
}

pub(super) fn write_controller(stage: &Path) -> Result<PathBuf, NpmRuntimeError> {
    let path = stage.join("npm-controller.js");
    io::write_private_file(&path, include_bytes!("npm_controller.js"))?;
    Ok(path)
}

pub(super) fn remove_stage_file(stage: &Path, name: &str) -> Result<(), NpmRuntimeError> {
    if stage
        .file_name()
        .and_then(|name| name.to_str())
        .is_none_or(|name| !name.starts_with("stage-"))
        || !matches!(name, "npm-controller.js")
    {
        return Err(NpmRuntimeError::StateConflict);
    }
    fs::remove_file(stage.join(name)).map_err(|_| NpmRuntimeError::StateConflict)
}

pub(super) fn cleanup_stage(registry: &Registry, stage: &Path) -> Result<(), NpmRuntimeError> {
    let root = registry.npm_input_root().map_err(registry_error)?;
    if stage.parent() != Some(root.as_path())
        || stage
            .file_name()
            .and_then(|name| name.to_str())
            .is_none_or(|name| !name.starts_with("stage-"))
    {
        return Err(NpmRuntimeError::StateConflict);
    }
    io::ensure_private_dir(stage)?;
    fs::remove_dir_all(stage).map_err(|_| NpmRuntimeError::Unknown)?;
    Ok(())
}

pub(super) fn cleanup_abandoned_stages(registry: &Registry) -> Result<(), NpmRuntimeError> {
    let root = registry.npm_input_root().map_err(registry_error)?;
    for entry in fs::read_dir(&root).map_err(|_| NpmRuntimeError::StateConflict)? {
        let entry = entry.map_err(|_| NpmRuntimeError::StateConflict)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(NpmRuntimeError::StateConflict);
        };
        if name.starts_with("stage-") {
            let path = entry.path();
            io::ensure_private_dir(&path)?;
            fs::remove_dir_all(path).map_err(|_| NpmRuntimeError::Unknown)?;
        }
    }
    Ok(())
}

pub(super) fn bundle_manifest_bytes(
    binding: &NpmInputBinding,
    entries: Vec<(u64, String)>,
) -> Result<Vec<u8>, NpmRuntimeError> {
    if entries.len() != binding.tarballs().len() {
        return Err(NpmRuntimeError::StateConflict);
    }
    let tarballs = binding
        .tarballs()
        .iter()
        .zip(entries)
        .enumerate()
        .map(|(index, (package, (bytes, sha512)))| BundleTarball {
            package_path: package.package_path().to_owned(),
            resolved_url: package.resolved_url().to_owned(),
            file: format!("tarballs/{index:04}.tgz"),
            bytes,
            sha512,
        })
        .collect();
    serde_json::to_vec(&BundleManifest {
        version: 1,
        input_key_sha256: binding.input_key_sha256().to_owned(),
        manifest_sha256: binding.manifest_sha256().to_owned(),
        lock_sha256: binding.lock_sha256().to_owned(),
        tarballs,
    })
    .map_err(|_| NpmRuntimeError::Failed)
}

pub(super) fn write_bundle_manifest(stage: &Path, bytes: &[u8]) -> Result<(), NpmRuntimeError> {
    io::write_private_file(&stage.join("bundle.json"), bytes)
}

pub(super) fn bundle_sha256(manifest: &[u8]) -> String {
    io::hex(&Sha256::digest(manifest))
}

pub(super) fn effective_toolchain_id(input_key: &str, bundle_sha256: &str) -> String {
    super::npm_lock::effective_toolchain_id(input_key, bundle_sha256)
}

pub(super) fn load_verified(
    registry: &Registry,
    binding: &NpmInputBinding,
) -> Result<Option<VerifiedBundle>, NpmRuntimeError> {
    let input_root = registry.npm_input_root().map_err(registry_error)?;
    let path = input_root.join(binding.input_key_sha256());
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(NpmRuntimeError::StateConflict),
        Ok(metadata) if !io::is_private_dir(&metadata) => {
            return Err(NpmRuntimeError::StateConflict);
        }
        Ok(_) => {}
    }
    verify::verify_directory(&path, binding).map(Some)
}

pub(super) fn validate_stage_tarballs(
    stage: &Path,
    binding: &NpmInputBinding,
) -> Result<Vec<(u64, String)>, NpmRuntimeError> {
    let mut entries = Vec::with_capacity(binding.tarballs().len());
    let mut total = 0u64;
    for (index, package) in binding.tarballs().iter().enumerate() {
        let path = stage.join(format!("tarballs/{index:04}.tgz"));
        let (bytes, sha512) = verify::verify_stage_tarball(&path, package.sha512_integrities())?;
        total = total
            .checked_add(bytes)
            .ok_or(NpmRuntimeError::TooManyPackages)?;
        if total > MAX_BUNDLE_BYTES {
            return Err(NpmRuntimeError::TooManyPackages);
        }
        entries.push((bytes, sha512));
    }
    Ok(entries)
}

fn registry_error(error: RegistryError) -> NpmRuntimeError {
    match error {
        RegistryError::Conflict | RegistryError::InvalidStateRoot => NpmRuntimeError::StateConflict,
        RegistryError::Io => NpmRuntimeError::Failed,
        #[cfg(unix)]
        RegistryError::Busy => NpmRuntimeError::Failed,
    }
}
