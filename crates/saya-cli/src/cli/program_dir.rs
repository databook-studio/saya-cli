//! The default trusted program directory a coding launch searches for
//! installed programs when `--program-dir` is not stated.

use std::path::PathBuf;

/// The trusted program directory a launch uses when none is stated.
///
/// Unix has one conventional answer. Windows has no `/usr/bin`, and a rooted
/// path without a drive there names a directory on whichever drive the
/// process happens to be using, not an absolute location: canonicalising
/// `/usr/bin` from a process working on `C:` asks for `C:\usr\bin`, which a
/// stock host does not have. The system directory is the trusted,
/// always-present equivalent.
pub(crate) fn default_program_dir() -> PathBuf {
    #[cfg(windows)]
    {
        // `SystemRoot` is the conventional source; a caller can influence it,
        // but so can it state `--program-dir`. A non-absolute value is not
        // trusted as a location, and the conventional fallback stands in.
        let root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .filter(|root| root.is_absolute())
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        root.join("System32")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/usr/bin")
    }
}
