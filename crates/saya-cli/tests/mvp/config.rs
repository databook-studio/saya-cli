use crate::common::*;
use std::fs;
use std::path::Path;

/// Runs the saya binary with every on-disk pointer isolated under `root`:
/// config home, state db, HOME (so the default state path can't reach the
/// developer's real `~/Library/Application Support`), and cwd. `saya_process`
/// only isolates config home; `ask` also opens a state db, so the ask-based
/// tests below need this fuller isolation.
fn saya_isolated(root: &Path, args: &[&str]) -> std::process::Output {
    saya_isolated_env(root, args, &[])
}

/// `saya_isolated` plus extra environment variables, e.g. a `SAYA_MODEL`
/// override for the `--explain` layer tests.
fn saya_isolated_env(root: &Path, args: &[&str], envs: &[(&str, &str)]) -> std::process::Output {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args(args)
        .current_dir(root)
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("HOME", root.join("home"));
    for (name, value) in envs {
        command.env(name, value);
    }
    command.output().unwrap()
}

include!("config/init_and_discovery.rs");

// ---------------------------------------------------------------------------
// a first run that ends somewhere. The cold path (empty config home, no
//.saya) used to land a new user on three unrelated failures, none naming a
// next step, and the third was self-inflicted: `config init` wrote only to the
// untrusted project layer, so the next command warned that it had ignored the
// templates init itself just wrote. These tests pin the post-fix behaviour:
// init writes the trusted user layer by default, `--project` keeps the old
// project-layer path, doctor advises (and exits non-zero when the setup cannot
// work), and the three failure paths each name an actionable next command.
// ---------------------------------------------------------------------------

include!("config/init_and_startup.rs");

// ---------------------------------------------------------------------------
// `config doctor --task <query|ai|coding>`: one task's readiness as
// structured checks, exit 0 when ready / 3 when not, computed from the
// resolved configuration only. No task flag keeps the flat report and exit
// code unchanged — the five doctor tests above pin that.
// ---------------------------------------------------------------------------

include!("config/doctor_tasks.rs");

// ---------------------------------------------------------------------------
// `config show --explain`: which layer supplied each setting. Plain
// `config show` stays byte-identical — the tests below pin the default-layer
// vocabulary, the user-file and environment layers with their paths, the one
// JSON object shape, and the doctor `fix:` hints. Values are redacted exactly
// as without the flag, so no test here ever sees a secret value.
// ---------------------------------------------------------------------------

include!("config/theme_precedence.rs");

include!("config/layer_and_hints.rs");
