//! The CPU a child process has used, from the OS's own accounting.
//!
//! `getrusage(RUSAGE_CHILDREN)` counts a child only once it has been reaped, so
//! it cannot say what a *live* child is costing — and the idle measure is
//! exactly that question. The per-process figure is the same number reached by
//! the other door: `/proc/<pid>/stat` on Linux, `ps -o time=` elsewhere on a
//! Unix host. Windows has no measure here, and the harness reports the window
//! as skipped there rather than guessing.

use std::time::Duration;

/// The CPU `pid` has used so far: user plus system time.
#[cfg(unix)]
pub(crate) fn cpu_time(pid: u32) -> Option<Duration> {
    #[cfg(target_os = "linux")]
    {
        linux_cpu_time(pid)
    }
    #[cfg(not(target_os = "linux"))]
    {
        ps_cpu_time(pid)
    }
}

/// The CPU `pid` used over `window`, as a percentage of the window.
#[cfg(unix)]
pub(crate) fn idle_percent(pid: u32, window: Duration) -> Option<f64> {
    let before = cpu_time(pid)?;
    std::thread::sleep(window);
    let after = cpu_time(pid)?;
    Some(percent(after.saturating_sub(before), window))
}

/// `used` as a percentage of `window`.
pub(crate) fn percent(used: Duration, window: Duration) -> f64 {
    used.as_secs_f64() / window.as_secs_f64() * 100.0
}

#[cfg(target_os = "linux")]
fn linux_cpu_time(pid: u32) -> Option<Duration> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `/proc/<pid>/stat` puts the command in parentheses — and it may itself
    // contain spaces or a `)` — so the fields are read after the last `)`.
    // What follows starts at state (field 3), which makes utime (14) and stime
    // (15) the eleventh and twelfth entries of it.
    let rest = stat.rsplit_once(')')?.1;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    let ticks = clock_ticks()?;
    Some(Duration::from_secs_f64(
        (utime + stime) as f64 / ticks as f64,
    ))
}

#[cfg(target_os = "linux")]
fn clock_ticks() -> Option<u64> {
    // SAFETY: `sysconf` reads a process-wide constant and cannot fail here.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    u64::try_from(ticks).ok().filter(|ticks| *ticks > 0)
}

#[cfg(all(unix, not(target_os = "linux")))]
fn ps_cpu_time(pid: u32) -> Option<Duration> {
    let output = std::process::Command::new("ps")
        .args(["-o", "time=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    parse_ps_time(String::from_utf8_lossy(&output.stdout).trim())
}

/// The `[[dd-]hh:]mm:ss[.cc]` form `ps -o time=` prints, in seconds.
#[cfg(all(unix, not(target_os = "linux")))]
fn parse_ps_time(text: &str) -> Option<Duration> {
    let (days, rest) = match text.split_once('-') {
        Some((days, rest)) => (days.trim().parse::<f64>().ok()?, rest),
        None => (0.0, text),
    };
    let parts: Vec<&str> = rest.split(':').map(str::trim).collect();
    let number = |text: &&str| text.parse::<f64>().ok();
    let seconds: f64 = match parts.as_slice() {
        [hours, minutes, seconds] => {
            number(hours)? * 3600.0 + number(minutes)? * 60.0 + number(seconds)?
        }
        [minutes, seconds] => number(minutes)? * 60.0 + number(seconds)?,
        [seconds] => number(seconds)?,
        _ => return None,
    };
    Some(Duration::from_secs_f64(days * 86_400.0 + seconds))
}
