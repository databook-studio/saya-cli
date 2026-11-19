//! The plugin's activation rule: which surfaces have a database side at all.
//!
//! The rule keys on the surface and the run's resolved profile count — the
//! chat activates only with a profile, every run a job started activates
//! regardless — so it is tested here without a run loop, because the chat is
//! not wired through the registry yet.

use saya_agent::{RunContext, RunPlugin, RunSurface};

use super::tests::chat_plugin;

/// A run of `surface` resolved with `database_profiles` database profiles.
const fn run(surface: RunSurface, database_profiles: usize) -> RunContext {
    RunContext {
        surface,
        database_profiles,
    }
}

/// A chat with no profile has nothing to read and nothing to ask, so its
/// database plugin stays out of the run; one profile is the whole condition.
#[test]
fn chat_activates_only_with_a_database_profile() {
    let plugin = chat_plugin();

    assert!(!plugin.activate(&run(RunSurface::Chat, 0)));
    assert!(plugin.activate(&run(RunSurface::Chat, 1)));
    assert!(plugin.activate(&run(RunSurface::Chat, 3)));
}

/// Every surface a job starts activates even with no profile selected: the job
/// already decided its database scope, and the ask needs no profile at all.
#[test]
fn a_job_started_surface_activates_without_a_profile() {
    let plugin = chat_plugin();

    for surface in [
        RunSurface::Headless,
        RunSurface::Background,
        RunSurface::Helper,
    ] {
        assert!(plugin.activate(&run(surface, 0)), "{surface:?}");
    }
}
