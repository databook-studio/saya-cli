use super::*;

pub(super) fn run() -> ProbeReport {
    ProbeReport {
        platform: "windows",
        kernel: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        checks: vec![
            Check::info(
                "windows_probe_scope",
                "the probe records findings, it does not enforce anything; on \
                     Windows there is nothing to measure",
            ),
            Check::failed(
                "sandbox_available",
                true,
                "explicit recorded result: Windows has no OS sandbox primitive this \
                     design targets — no Seatbelt, no Landlock, no supported equivalent. \
                     Per U9 the runner fails closed by construction on Windows: this check \
                     can never pass, the runner tool is never registered, and the 3-OS CI \
                     matrix stays green with the runner simply absent.",
            ),
        ],
    }
}
