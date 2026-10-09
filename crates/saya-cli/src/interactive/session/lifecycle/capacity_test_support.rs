//! Shared oversized-session fixtures for the capacity tests: states whose
//! persisted record lands in a chosen capacity band, and throwaway store
//! roots for a real [`FsSessionStore`].

use crate::interactive::session::state::SessionState;

static FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A session of `turns` recorded turns, each assistant side `chars` bytes of
/// plain filler. A persisted turn is roughly `chars` bytes plus the JSON
/// shape around it, so `state_with_turns(id, 500, 11_500)` lands over the
/// 4 MiB cap and `state_with_turns(id, 300, 11_500)` lands between the
/// 3 MiB warn level and the cap — every test re-checks the band it needs.
pub(crate) fn state_with_turns(id: &str, turns: usize, chars: usize) -> SessionState {
    let mut state = SessionState::new(id, None, "test-model");
    for index in 0..turns {
        state.record_turn(format!("q{index}"), "x".repeat(chars), false, Vec::new());
    }
    state
}

/// A unique throwaway root for a real store.
pub(crate) fn temp_store_root(label: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-capacity-{label}-{}-{stamp}-{}",
        std::process::id(),
        FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ));
    std::fs::create_dir_all(&root).expect("create the capacity test root");
    root
}
