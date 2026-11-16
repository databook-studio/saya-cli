use std::path::Path;

/// Whether two paths name the same location. A POSIX Git prints the path it
/// resolved, so the two must match exactly; the Windows implementation
/// compares normalized spellings instead.
pub(super) fn same_location(left: &Path, right: &Path) -> bool {
    left == right
}
