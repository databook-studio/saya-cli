//! Program-directory selection for the coding fixture sandbox.
//!
//! The fixture hands the application a single trusted directory whose programs
//! it may run, so that directory must actually contain every program the
//! fixture's configured profiles invoke. macOS keeps `sleep` and `sh` in `/bin`
//! while `printf` and `false` live only in `/usr/bin`, but Linux distributions
//! differ, so instead of hard-coding one layout the fixture asks which
//! candidate directory holds all of the programs it needs.

use std::path::{Path, PathBuf};

/// Return the first candidate directory — `/bin`, then `/usr/bin` — that
/// contains every program in `programs` as a file, or panic when none does.
///
/// `PATH` is deliberately not consulted: the result becomes the sandbox's
/// trusted program directory, so it must name a real directory that covers
/// every requested program, not whatever happens to resolve through `PATH`.
pub(super) fn program_dir_for(programs: &[&str]) -> PathBuf {
    for directory in ["/bin", "/usr/bin"] {
        if programs
            .iter()
            .all(|program| Path::new(directory).join(program).is_file())
        {
            return PathBuf::from(directory);
        }
    }
    panic!(
        "no candidate program directory provides all of {:?}; checked /bin and /usr/bin",
        programs
    );
}
