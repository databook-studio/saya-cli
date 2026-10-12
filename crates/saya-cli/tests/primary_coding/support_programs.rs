//! Program-directory selection for the coding fixture sandbox.
//!
//! The fixture hands the application a single trusted directory whose programs
//! it may run, so that directory must actually contain every program the
//! fixture's configured profiles invoke. macOS keeps `sleep` and `sh` in `/bin`
//! while `printf` and `false` live only in `/usr/bin`, but Linux distributions
//! differ, so instead of hard-coding one layout the fixture asks which
//! candidate directory holds all of the programs it needs.

#[path = "../../../saya-harness/tests/support/exec_ready.rs"]
mod exec_ready;

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
            return real_programs(Path::new(directory), programs);
        }
    }
    panic!(
        "no candidate program directory provides all of {:?}; checked /bin and /usr/bin",
        programs
    );
}

/// The sandbox accepts a trusted program only when its path is its own
/// canonical path. Debian's merged-usr layout makes `/bin` a symlink and
/// `/bin/sh` a link to `dash`, so there the directory is replaced by one
/// holding real copies of exactly the requested programs; on macOS the
/// directory is returned unchanged.
fn real_programs(directory: &Path, programs: &[&str]) -> PathBuf {
    let canonical = |path: &Path| path.canonicalize().is_ok_and(|real| real == path);
    if programs
        .iter()
        .all(|program| canonical(&directory.join(program)))
    {
        return directory.to_path_buf();
    }
    let mut key = programs.to_vec();
    key.sort_unstable();
    let staged = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("saya-fixture-programs")
        .join(key.join("+"));
    std::fs::create_dir_all(&staged).expect("the fixture program directory is creatable");
    let staged = staged
        .canonicalize()
        .expect("the staged directory resolves");
    for program in programs {
        let target = staged.join(program);
        if target.is_file() {
            continue;
        }
        let source = directory
            .join(program)
            .canonicalize()
            .expect("the requested program resolves");
        // Copy beside the target and rename: concurrent test processes never
        // observe a partial executable, and a loser's rename just replaces
        // identical bytes.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let partial = staged.join(format!(
            ".{program}.{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::copy(&source, &partial).expect("the program copies");
        exec_ready::wait_until_exec_ready(&partial);
        std::fs::rename(&partial, &target).expect("the program installs");
    }
    staged
}
