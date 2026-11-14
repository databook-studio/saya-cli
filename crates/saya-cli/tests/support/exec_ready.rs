//! A freshly written executable is not always exec-safe. A thread that forks
//! while the file is open for writing leaves its child holding a write
//! descriptor until that child execs, and exec of the file then fails with
//! ETXTBSY. On Linux this waits until no process holds the file for writing;
//! elsewhere it returns at once.

#[cfg(target_os = "linux")]
use std::fs;
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub fn wait_until_exec_ready(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while held_for_writing(path) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(target_os = "linux")]
fn held_for_writing(path: &Path) -> bool {
    let Ok(processes) = fs::read_dir("/proc") else {
        return false;
    };
    processes.flatten().any(|process| {
        let fds = process.path().join("fd");
        fs::read_dir(&fds).is_ok_and(|entries| {
            entries.flatten().any(|fd| {
                fs::read_link(fd.path()).is_ok_and(|target| target == path)
                    && fs::read_to_string(process.path().join("fdinfo").join(fd.file_name()))
                        .ok()
                        .and_then(|info| {
                            let flags =
                                info.lines().find_map(|line| line.strip_prefix("flags:"))?;
                            u32::from_str_radix(flags.trim(), 8).ok()
                        })
                        .is_some_and(|flags| flags & 0o3 != 0)
            })
        })
    })
}

#[cfg(not(target_os = "linux"))]
fn held_for_writing(_path: &Path) -> bool {
    false
}
