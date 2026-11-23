//! The hook lane on Windows: the host lane composes there.
//!
//! A hook is the user's own command on the host lane. Windows has no OS
//! sandbox, so the lane is unsandboxed — but it is composed, exactly as it is
//! on unix, and the engine runs its hooks through the system command
//! processor. The unix hook fixtures stay `#[cfg(unix)]` because they are
//! `sh` scripts; this pins the composition the Windows lane has.

use saya_agent::CancellationToken;
use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput};
use saya_harness::runner::HostPolicy;

use super::{HookCaller, Hooks};

#[test]
fn a_host_only_hook_engine_composes_on_windows() {
    let file = ConfigFile::from_toml("[[hooks.stop]]\ncommand = \"echo done\"\n")
        .expect("the fixture's hook table is well formed");
    let resolved =
        saya_config::resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(file))
            .expect("the fixture's config resolves");
    assert!(!resolved.hooks.is_empty(), "the fixture must state a hook");
    let engine = Hooks::new(
        resolved.hooks,
        &std::env::temp_dir(),
        HostPolicy::default(),
        // A session that has no record id, the caller a `saya` session states.
        HookCaller::Session(None),
        CancellationToken::new(),
    )
    .expect("the host lane composes on Windows");
    drop(engine);
}
