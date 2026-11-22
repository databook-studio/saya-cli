//! `/reload`'s swap: a changed file moves the configuration in force, and a
//! broken one leaves the previous configuration exactly where it was.
//!
//! The re-read is the crate's one reload core, so what is asserted here is the
//! surface's half: which configuration a read that starts *after* a reload runs
//! on, and that a failed reload swaps nothing at all.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::live::LiveRuntime;
use super::ops::Asked;
use super::reload::apply;
use crate::cli::GlobalOptions;
use crate::config::reload::ReloadInputs;
use crate::config::runtime::{RuntimeConfig, load_with_sources};
use crate::interactive::SessionState;

/// The launched configuration, on its own tree: `max_rows = 10`, and the user
/// directory every re-read reads.
fn launched(label: &str) -> (PathBuf, Arc<RuntimeConfig>) {
    let root = root(label);
    let user = root.join("user");
    std::fs::create_dir_all(&user).expect("the fixture's user config dir is made");
    std::fs::write(user.join("config.toml"), CONFIG_10).expect("the launched config writes");
    let runtime = load_with_sources(&GlobalOptions::default(), &root, &user, BTreeMap::new())
        .expect("the launched configuration loads");
    (root, Arc::new(runtime))
}

/// A tree under the workspace's own `target/`, named for the test and the
/// process, so two runs never share one.
fn root(label: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate sits two levels under the workspace root")
        .join("target")
        .join(format!("tmp-reload-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the fixture's tree is made");
    root
}

/// The same launch inputs a `/reload` re-reads: no flags, this tree's own
/// directories, and no environment.
fn inputs(root: &Path) -> ReloadInputs {
    ReloadInputs {
        options: GlobalOptions::default(),
        cwd: root.to_path_buf(),
        user_dir: root.join("user"),
        process_env: BTreeMap::new(),
    }
}

const CONFIG_10: &str = "[ai]\nmodel = 'smoke'\n\n[run]\nmax_rows = 10\n";
const CONFIG_5: &str = "[ai]\nmodel = 'smoke'\n\n[run]\nmax_rows = 5\n";

fn state() -> SessionState {
    SessionState::new("s1", None, "smoke")
}

#[test]
fn a_broken_file_keeps_the_previous_runtime_in_use() {
    let (root, launched) = launched("broken");
    let live = LiveRuntime::new(Arc::clone(&launched));
    std::fs::write(root.join("user").join("config.toml"), "[run\n")
        .expect("the broken config writes");

    let asked = apply(&inputs(&root), &live, &mut state());

    let Asked::Error(text) = asked else {
        panic!("a broken file is a refusal, not a reload: {asked:?}");
    };
    assert!(
        text.contains("previous configuration is still in use"),
        "the pinned sentence is the answer: {text}"
    );
    // Nothing moved: the same configuration is still in force, and it is the
    // very one the launch resolved.
    assert!(
        Arc::ptr_eq(&live.current(), &launched),
        "a failed reload swaps nothing"
    );
    assert_eq!(
        live.current().resolved.max_rows,
        10,
        "the previous value is still in use"
    );
}

#[test]
fn a_changed_file_is_in_force_for_the_next_read() {
    let (root, launched) = launched("changed");
    let live = LiveRuntime::new(Arc::clone(&launched));
    std::fs::write(root.join("user").join("config.toml"), CONFIG_5)
        .expect("the edited config writes");

    let asked = apply(&inputs(&root), &live, &mut state());

    let Asked::Notice(text) = asked else {
        panic!("a valid file reloads: {asked:?}");
    };
    assert!(
        text.contains("max_rows: 10 -> 5"),
        "the plan names the change: {text}"
    );
    // The next read runs on the new configuration, and the one that was in
    // force before is untouched: a read that started earlier keeps its own.
    assert_eq!(live.current().resolved.max_rows, 5);
    assert_eq!(launched.resolved.max_rows, 10);
    assert!(
        !Arc::ptr_eq(&live.current(), &launched),
        "the swap put a new configuration in force"
    );
}
