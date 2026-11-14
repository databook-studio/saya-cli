use super::*;
use std::borrow::Cow;

/// Accepts an absolute path that resolves under the canonical `root` by
/// stripping the root. Any other absolute path is refused, naming the root.
/// A relative path is returned unchanged, so every containment check still
/// runs on it: `..` in an accepted absolute path is left for the component
/// walk to refuse, and symlinks are still refused at every component.
pub fn relative_argument<'a>(root: &Path, rel: &'a str) -> Result<Cow<'a, str>, HarnessError> {
    let path = Path::new(rel);
    if !path.is_absolute() {
        return Ok(Cow::Borrowed(rel));
    }
    let stripped = path
        .strip_prefix(root)
        .map_err(|_| HarnessError::PathOutsideRoot {
            path: rel.to_string(),
            root: root.display().to_string(),
        })?;
    let text = stripped.to_str().ok_or_else(|| HarnessError::InvalidPath {
        path: rel.to_string(),
    })?;
    // A Windows absolute path is stripped with its own separators; the
    // component walk speaks `/`, so hand it that spelling.
    #[cfg(windows)]
    return Ok(Cow::Owned(text.replace('\\', "/")));
    #[cfg(not(windows))]
    Ok(Cow::Borrowed(text))
}

impl Workspace {
    /// Validates `rel` and resolves it under the canonical root: argument
    /// validation and a component walk that refuses symlinks (creating missing
    /// directories only when `allow_create`). On unix this is the anchored
    /// walk — each component resolved against the previous descriptor — and
    /// the returned path is the walk's own record; elsewhere it is the
    /// component-by-path walk plus the canonicalise-then-prefix check.
    /// `pub(crate)` so the contained walk (`walk`) can re-run the exact same
    /// check on every candidate it reports — the one validator, reused, not
    /// re-derived.
    pub(crate) fn target(&self, rel: &str, allow_create: bool) -> Result<PathBuf, HarnessError> {
        #[cfg(unix)]
        return Ok(self.anchor(rel, allow_create)?.path().to_path_buf());

        #[cfg(not(unix))]
        {
            let rel = relative_argument(&self.root, rel)?;
            let rel = rel.as_ref();
            let comps = argument_components(&self.root, rel)?;
            let mut path = self.root.clone();
            let mut depth = 0usize;
            let mut final_exists = true;
            for (index, comp) in comps.iter().enumerate() {
                let last = index + 1 == comps.len();
                if comp == ".." {
                    if depth == 0 {
                        return Err(HarnessError::PathOutsideRoot {
                            path: rel.to_string(),
                            root: self.root.display().to_string(),
                        });
                    }
                    depth -= 1;
                    path.pop();
                    continue;
                }
                path.push(comp);
                match fs::symlink_metadata(&path) {
                    Ok(meta) if meta.file_type().is_symlink() => {
                        return Err(HarnessError::SymlinkRefused {
                            path: rel.to_string(),
                        });
                    }
                    Ok(meta) => {
                        if !last {
                            if !meta.is_dir() {
                                return Err(HarnessError::InvalidPath {
                                    path: rel.to_string(),
                                });
                            }
                            depth += 1;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        if !last {
                            if !allow_create {
                                return Err(HarnessError::NotFound {
                                    path: rel.to_string(),
                                });
                            }
                            create_dir_component(&path, rel)?;
                            depth += 1;
                        } else {
                            final_exists = false;
                        }
                    }
                    Err(error) => return Err(io_error("scan workspace path", &path, error)),
                }
            }
            self.prefix_check(&path, rel, final_exists)?;
            Ok(path)
        }
    }
    /// Canonicalise-then-prefix: the deepest anchor that exists — the path
    /// itself, or its parent when the final component is still to be
    /// created — must resolve under the canonical root. The unix walk is
    /// anchored by construction (each component resolved against the previous
    /// descriptor); this is the non-unix belt.
    #[cfg(not(unix))]
    fn prefix_check(&self, path: &Path, rel: &str, exists: bool) -> Result<(), HarnessError> {
        let anchor = if exists {
            path.to_path_buf()
        } else {
            path.parent().unwrap_or(self.root.as_path()).to_path_buf()
        };
        let canonical = fs::canonicalize(&anchor)
            .map_err(|error| io_error("canonicalise workspace path", &anchor, error))?;
        if !canonical.starts_with(&self.root) {
            return Err(HarnessError::PathOutsideRoot {
                path: rel.to_string(),
                root: self.root.display().to_string(),
            });
        }
        Ok(())
    }
    /// The workspace-relative spelling of `rel`: an absolute path under the
    /// canonical root is accepted and the root stripped, so a model that
    /// always sends absolute paths reaches the same file. Every containment
    /// check still runs on the result.
    pub fn relative_argument<'a>(&self, rel: &'a str) -> Result<Cow<'a, str>, HarnessError> {
        relative_argument(&self.root, rel)
    }
}
/// The argument half of [`Workspace::target`] with no filesystem contact:
/// argument validation plus the `..`-depth walk, assuming every non-`..`
/// component is a directory — the conservative shape check that refuses an
/// escape (absolute forms, `..` rising above the root, drive shapes) before
/// any scan or creation can happen. A caller that must refuse a path before
/// touching the disk runs this first and then `target`.
pub(crate) fn validate_argument(root: &Path, rel: &str) -> Result<(), HarnessError> {
    let comps = argument_components(root, rel)?;
    let mut depth = 0usize;
    for comp in comps {
        if comp == ".." {
            if depth == 0 {
                return Err(HarnessError::PathOutsideRoot {
                    path: rel.to_string(),
                    root: root.display().to_string(),
                });
            }
            depth -= 1;
        } else {
            depth += 1;
        }
    }
    Ok(())
}
pub(crate) fn argument_components(root: &Path, rel: &str) -> Result<Vec<String>, HarnessError> {
    let invalid = || HarnessError::InvalidPath {
        path: rel.to_string(),
    };
    let escaped = || HarnessError::PathOutsideRoot {
        path: rel.to_string(),
        root: root.display().to_string(),
    };
    if rel.is_empty()
        || rel.len() > MAX_IO_BYTES
        || rel.as_bytes().contains(&0)
        || rel.contains('\\')
    {
        return Err(invalid());
    }
    if rel.starts_with('/') {
        return Err(escaped());
    }
    // `.` is a no-op component and a trailing slash is a directory
    // spelling (`src/`, `./src`). Models send both; neither escapes the
    // root. An empty *middle* component (`src//foo`) stays invalid.
    let raw: Vec<&str> = rel.split('/').collect();
    let raw = match raw.split_last() {
        Some((&"", rest)) => rest,
        _ => raw.as_slice(),
    };
    let comps: Vec<String> = raw
        .iter()
        .copied()
        .filter(|comp| *comp != ".")
        .map(str::to_string)
        .collect();
    if comps.is_empty() || comps.iter().any(String::is_empty) {
        return Err(invalid());
    }
    if comps[0].len() == 2 && comps[0].ends_with(':') {
        return Err(invalid());
    }
    // `.git` is NOT special-cased here: a working checkout's git
    // metadata is ordinary content, so an edit or a shell may write it. Every
    // containment rule that is about escaping the root — the `..`-depth walk
    // above, absolute forms, drive shapes — still refuses.
    Ok(comps)
}
#[cfg(not(unix))]
fn create_dir_component(path: &Path, rel: &str) -> Result<(), HarnessError> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(_) => {
            // Lost a creation race; the winner must be a real directory.
            let meta = fs::symlink_metadata(path)
                .map_err(|error| io_error("scan workspace path", path, error))?;
            if meta.file_type().is_symlink() {
                return Err(HarnessError::SymlinkRefused {
                    path: rel.to_string(),
                });
            }
            if !meta.is_dir() {
                return Err(HarnessError::InvalidPath {
                    path: rel.to_string(),
                });
            }
            Ok(())
        }
    }
}
