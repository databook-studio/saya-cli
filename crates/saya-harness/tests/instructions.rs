//! The bounded instruction loader battery: at most two named files are ever
//! read — never through a symlink, never a discovery walk, never an import —
//! with caps, truncation, and whole-file identity honoured as data.

#[path = "instructions/caps.rs"]
mod caps;
#[path = "instructions/discovery.rs"]
mod discovery;
#[path = "instructions/isolation.rs"]
mod isolation;
#[path = "instructions/refusals.rs"]
mod refusals;

use std::{fs, path::PathBuf};

use saya_harness::instructions::{
    InstructionLimits, InstructionOrigin, InstructionSource, LoadedInstructions, load_instructions,
};

const CONTENT: &[u8] = b"inside-sentinel";
const KNOWN_DIGEST: &str = "b568f317d963edff551663a3b460d49899fa97a2d313bb50078916da9e74d15f";
const OUTSIDE: &[u8] = b"outside-sentinel-MUST-NOT-LEAK";

const LIMITS: InstructionLimits = InstructionLimits {
    project_max_bytes: 32 * 1024,
    personal_max_bytes: 16 * 1024,
    total_max_bytes: 48 * 1024,
};

struct Sandbox {
    outer: PathBuf,
    project: PathBuf,
    personal: PathBuf,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-instructions-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        let project = outer.join("project");
        let personal = outer.join("personal");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&personal).unwrap();
        Self {
            outer,
            project,
            personal,
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

fn write_project(sandbox: &Sandbox, bytes: &[u8]) {
    fs::write(sandbox.project.join("AGENTS.md"), bytes).unwrap();
}

fn write_personal(sandbox: &Sandbox, bytes: &[u8]) {
    fs::write(sandbox.personal.join("instructions.md"), bytes).unwrap();
}

fn load(sandbox: &Sandbox, limits: &InstructionLimits) -> LoadedInstructions {
    load_instructions(Some(&sandbox.project), Some(&sandbox.personal), limits)
}

fn source_of(loaded: &LoadedInstructions, origin: InstructionOrigin) -> &InstructionSource {
    loaded
        .sources
        .iter()
        .find(|source| source.origin == origin)
        .expect("source must be present")
}

fn no_leak(loaded: &LoadedInstructions) {
    let sentinel = std::str::from_utf8(OUTSIDE).unwrap();
    for source in &loaded.sources {
        assert!(
            !source.body.contains(sentinel),
            "outside sentinel must never appear in a body"
        );
    }
}
