use std::path::Path;

/// Whether two paths name the same location. Git for Windows prints
/// `C:/dir` (forward slashes, no verbatim prefix) while `canonicalize`
/// returns `\\?\C:\dir`, and the Git service stores the simplified `C:\dir`;
/// only the spelling differs. Normalizing both still refuses a path that
/// resolves elsewhere, such as a symlinked worktree, because its canonical
/// path names a different location.
pub(super) fn same_location(left: &Path, right: &Path) -> bool {
    let (Some(left), Some(right)) = (left.to_str(), right.to_str()) else {
        return false;
    };
    normalized(left).eq_ignore_ascii_case(&normalized(right))
}

fn normalized(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let path = match path.strip_prefix(r"UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => path.to_owned(),
    };
    path.replace('\\', "/")
}
