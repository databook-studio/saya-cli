use super::*;

#[cfg(unix)]
pub(crate) fn identity_of(metadata: &fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}
/// The identity of an open file for non-unix verification paths. Windows
/// uses the object identifier held by the kernel, not creation metadata that
/// an overwrite may preserve on the destination.
#[cfg(not(unix))]
pub(crate) fn file_identity(file: &fs::File) -> std::io::Result<(u64, u64)> {
    #[cfg(windows)]
    {
        super::windows_identity::identity(file)
    }
    #[cfg(not(windows))]
    {
        file.metadata().map(|metadata| identity_of(&metadata))
    }
}
#[cfg(not(unix))]
pub(crate) fn path_identity(path: &Path) -> std::io::Result<(u64, u64)> {
    let file = open_identity_path(path)?;
    file_identity(&file)
}
#[cfg(not(unix))]
fn open_identity_path(path: &Path) -> std::io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    if super::windows_identity::is_reparse_point(&file)? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "workspace identity path is a reparse point",
        ));
    }
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "workspace identity path is not a regular file",
        ));
    }
    Ok(file)
}
/// Metadata identity for non-unix pre-open checks. Windows commit paths use
/// [`file_identity`] instead, because a replacement can retain the
/// destination's creation metadata.
#[cfg(all(not(unix), not(windows)))]
pub(crate) fn identity_of(metadata: &fs::Metadata) -> (u64, u64) {
    (metadata.len(), 0)
}
/// Atomic replace for the non-unix commit paths, shared with the range
/// patch. `rename` replaces an existing destination on Windows, preserving
/// the temporary file's identity for the post-commit verification.
#[cfg(not(unix))]
pub(crate) fn replace_workspace_file(temp: &Path, target: &Path) -> Result<(), HarnessError> {
    replace_file(temp, target)
}
#[cfg(not(unix))]
fn replace_file(temp: &Path, target: &Path) -> Result<(), HarnessError> {
    fs::rename(temp, target).map_err(|error| io_error("replace workspace file", target, error))
}
pub(crate) fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default()
}
