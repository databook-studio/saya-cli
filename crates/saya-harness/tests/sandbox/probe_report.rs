use super::*;

/// The registration evidence, written where a human reads it: the probe
/// report and the exact profile the run would spawn under.
#[cfg(target_os = "macos")]
#[test]
fn the_probe_report_is_written_for_the_run() {
    let (_root, provision) = proven_spawn("report", vec![]);
    let dir = std::env::var("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let _ = fs::write(
        dir.join("sandbox-startup-report.txt"),
        provision.report().render(),
    );
    let spawn = provision.spawn().expect("proven");
    if let Some(profile) = spawn.profile() {
        let _ = fs::write(dir.join("sandbox-startup-profile.sb"), profile);
    }
}
