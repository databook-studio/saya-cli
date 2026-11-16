//! Linux's defined behaviour for host confinement: the probe fails closed by
//! design, so the lanes that need a proven sandbox are not offered. This is
//! the counterpart to the macOS-only confinement suites — the refusal is
//! typed, and a refusal never yields an unconfined command.
#![cfg(target_os = "linux")]

use std::path::Path;

use saya_harness::runner::sandbox::{RunSandbox, SandboxProvision};

use crate::support::TempRoot;

#[test]
fn the_sandbox_lane_is_not_offered_without_proven_confinement() {
    let temp = TempRoot::new("linux-confinement");
    let root = temp.path().join("checkout");
    std::fs::create_dir(&root).unwrap();
    let sandbox = RunSandbox::new([root], std::iter::empty()).unwrap();
    let provision = sandbox
        .prepare(Path::new("/bin"))
        .expect("the probe answers rather than erroring");
    assert!(
        matches!(provision, SandboxProvision::Refused(_)),
        "the Linux probe fails closed: no sandbox lane is offered"
    );
    assert!(
        provision.spawn().is_none(),
        "a refusal never yields a spawn"
    );
}
