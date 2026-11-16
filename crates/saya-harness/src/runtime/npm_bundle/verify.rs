//! Whole-bundle shape, file identity, size, and SRI verification.

use std::{collections::BTreeSet, fs, io::Read, path::Path};

use sha2::{Digest as _, Sha512};

use super::super::{npm::NpmRuntimeError, npm_lock::NpmInputBinding};
use super::{
    BundleManifest, MAX_BUNDLE_BYTES, MAX_MANIFEST_BYTES, VerifiedBundle,
    io::{self, ensure_private_dir},
};

pub(super) fn verify_directory(
    path: &Path,
    binding: &NpmInputBinding,
) -> Result<VerifiedBundle, NpmRuntimeError> {
    ensure_private_dir(path)?;
    let manifest_bytes = io::read_private_file(&path.join("bundle.json"), MAX_MANIFEST_BYTES)?;
    let manifest: BundleManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| NpmRuntimeError::StateConflict)?;
    if manifest.version != 1
        || manifest.input_key_sha256 != binding.input_key_sha256()
        || manifest.manifest_sha256 != binding.manifest_sha256()
        || manifest.lock_sha256 != binding.lock_sha256()
        || manifest.tarballs.len() != binding.tarballs().len()
    {
        return Err(NpmRuntimeError::StateConflict);
    }
    if io::read_private_file(&path.join("package.json"), 256 * 1024)? != binding.manifest_bytes()
        || io::read_private_file(&path.join("package-lock.json"), MAX_MANIFEST_BYTES)?
            != binding.lock_bytes()
    {
        return Err(NpmRuntimeError::StateConflict);
    }
    verify_names(path, &manifest)?;
    ensure_private_dir(&path.join("tarballs"))?;
    let mut total = 0u64;
    for (index, (entry, package)) in manifest.tarballs.iter().zip(binding.tarballs()).enumerate() {
        if entry.package_path != package.package_path()
            || entry.resolved_url != package.resolved_url()
            || entry.file != format!("tarballs/{index:04}.tgz")
        {
            return Err(NpmRuntimeError::StateConflict);
        }
        let (bytes, digest) =
            verify_tarball(&path.join(&entry.file), package.sha512_integrities())?;
        if bytes != entry.bytes || digest != entry.sha512 {
            return Err(NpmRuntimeError::StateConflict);
        }
        total = total
            .checked_add(bytes)
            .ok_or(NpmRuntimeError::StateConflict)?;
        if total > MAX_BUNDLE_BYTES {
            return Err(NpmRuntimeError::StateConflict);
        }
    }
    Ok(VerifiedBundle {
        path: path.to_path_buf(),
        bundle_sha256: super::bundle_sha256(&manifest_bytes),
        tarball_count: manifest.tarballs.len(),
        tarball_bytes: total,
    })
}

pub(super) fn verify_stage_tarball(
    path: &Path,
    expected: &[[u8; 64]],
) -> Result<(u64, String), NpmRuntimeError> {
    verify_tarball(path, expected)
}

fn verify_names(path: &Path, manifest: &BundleManifest) -> Result<(), NpmRuntimeError> {
    let expected = BTreeSet::from([
        "bundle.json".to_owned(),
        "package.json".to_owned(),
        "package-lock.json".to_owned(),
        "tarballs".to_owned(),
    ]);
    let actual = names(path)?;
    if actual != expected {
        return Err(NpmRuntimeError::StateConflict);
    }
    let expected_files: BTreeSet<String> = manifest
        .tarballs
        .iter()
        .map(|tarball| {
            Path::new(&tarball.file)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
                .ok_or(NpmRuntimeError::StateConflict)
        })
        .collect::<Result<_, _>>()?;
    if names(&path.join("tarballs"))? != expected_files {
        return Err(NpmRuntimeError::StateConflict);
    }
    Ok(())
}

fn names(path: &Path) -> Result<BTreeSet<String>, NpmRuntimeError> {
    fs::read_dir(path)
        .map_err(|_| NpmRuntimeError::StateConflict)?
        .map(|entry| {
            entry
                .map_err(|_| NpmRuntimeError::StateConflict)?
                .file_name()
                .into_string()
                .map_err(|_| NpmRuntimeError::StateConflict)
        })
        .collect()
}

fn verify_tarball(path: &Path, expected: &[[u8; 64]]) -> Result<(u64, String), NpmRuntimeError> {
    let file = io::open_private_file(path, super::MAX_TARBALL_BYTES)?;
    let mut hasher = Sha512::new();
    let mut bytes = 0u64;
    let mut limited = file.take(super::MAX_TARBALL_BYTES + 1);
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let count = limited
            .read(&mut chunk)
            .map_err(|_| NpmRuntimeError::StateConflict)?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .ok_or(NpmRuntimeError::StateConflict)?;
        if bytes > super::MAX_TARBALL_BYTES {
            return Err(NpmRuntimeError::TooManyPackages);
        }
        hasher.update(&chunk[..count]);
    }
    let digest = hasher.finalize();
    if !expected
        .iter()
        .any(|candidate| candidate.as_slice() == digest.as_slice())
    {
        return Err(NpmRuntimeError::InvalidIntegrity);
    }
    Ok((bytes, io::hex(&digest)))
}
