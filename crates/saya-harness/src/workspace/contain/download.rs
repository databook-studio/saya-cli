use super::*;

impl Workspace {
    /// Opens the download machinery's partial file under containment: the same
    /// argument validation and component walk as any workspace write,
    /// then a no-follow open at 0600 — `truncate` for a fresh download,
    /// write-capable for a resume — with a post-open identity check so the
    /// file streamed into is the one that was scanned. Resume seeks after
    /// discarding an unaccounted tail. Never executable.
    pub(crate) fn open_download_part(
        &self,
        rel: &str,
        truncate: bool,
    ) -> Result<(PathBuf, fs::File), HarnessError> {
        #[cfg(unix)]
        return super::super::download::open_download_part(self, rel, truncate);

        #[cfg(not(unix))]
        {
            let path = self.target(rel, true)?;
            let mut options = OpenOptions::new();
            options.write(true).create(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt as _;
                // Rust 1.88 maps create+truncate to OPEN_ALWAYS and truncates
                // after opening; this is valid with OPEN_REPARSE_POINT, unlike
                // CreateFile's CREATE_ALWAYS disposition.
                options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
            }
            if truncate {
                options.truncate(true);
            }
            let file = options
                .open(&path)
                .map_err(|error| io_error("open workspace download part", &path, error))?;
            let opened = file
                .metadata()
                .map_err(|error| io_error("stat opened download part", &path, error))?;
            #[cfg(windows)]
            if super::windows_identity::is_reparse_point(&file)
                .map_err(|error| io_error("stat opened download part", &path, error))?
            {
                return Err(HarnessError::SymlinkRefused {
                    path: rel.to_string(),
                });
            }
            if opened.file_type().is_symlink() {
                return Err(HarnessError::SymlinkRefused {
                    path: rel.to_string(),
                });
            }
            if !opened.is_file() {
                return Err(HarnessError::NotRegularFile {
                    path: rel.to_string(),
                });
            }
            let current = fs::symlink_metadata(&path)
                .map_err(|error| io_error("verify opened download part", &path, error))?;
            if current.file_type().is_symlink() {
                return Err(HarnessError::SymlinkRefused {
                    path: rel.to_string(),
                });
            }
            if !current.is_file() {
                return Err(HarnessError::NotRegularFile {
                    path: rel.to_string(),
                });
            }
            let current_identity = path_identity(&path)
                .map_err(|error| io_error("verify opened download part", &path, error))?;
            let opened_identity = file_identity(&file)
                .map_err(|error| io_error("stat opened download part", &path, error))?;
            if current_identity != opened_identity {
                return Err(HarnessError::IdentityChanged {
                    path: rel.to_string(),
                });
            }
            Ok((path, file))
        }
    }
    /// Promotes a completed download part over `dest_rel` atomically under
    /// the destination guard (`download_guard`): create-if-absent with no
    /// expected digest; with one, replace only when the destination's
    /// current bytes hash to it — checked at the rename, under the commit
    /// gate. Both part and destination are anchored to the walked
    /// descriptors (unix) or resolved by the contained walk (elsewhere), and
    /// the destination is re-verified afterwards (regular file, 0600, never
    /// executable, still the inode that was renamed) so a completed download
    /// is never pretended onto a swapped path. Every refusal leaves the part
    /// in place.
    pub(crate) fn promote_download(
        &self,
        part_rel: &str,
        dest_rel: &str,
        expected_destination_sha256: Option<&str>,
    ) -> Result<(), HarnessError> {
        #[cfg(unix)]
        return super::super::download::promote(
            self,
            part_rel,
            dest_rel,
            expected_destination_sha256,
        );

        #[cfg(not(unix))]
        return super::download_guard::promote_by_path(
            self,
            part_rel,
            dest_rel,
            expected_destination_sha256,
        );
    }
    /// Removes a workspace file relative to its validated parent; absence is
    /// already-done, not an error. The sidecar removal's contained form.
    pub(crate) fn unlink(&self, rel: &str) -> Result<(), HarnessError> {
        #[cfg(unix)]
        return super::super::download::unlink(self, rel);

        #[cfg(not(unix))]
        {
            let path = self.target(rel, false)?;
            match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(io_error("remove download sidecar", &path, error)),
            }
        }
    }
}
