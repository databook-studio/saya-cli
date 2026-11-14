use super::*;

pub(super) fn run() -> ProbeReport {
    ProbeReport {
        platform: "other-unix",
        kernel: uname_line(),
        checks: vec![Check::failed(
            "sandbox_available",
            true,
            format!(
                "explicit recorded result: this platform ({}/{}) has no measured \
                     sandbox path in this design; fail closed by construction",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        )],
    }
}
