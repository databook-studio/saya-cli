//! Setup for the part-B database goldens: an assisted-memory REPL, the
//! product-CLI seed that puts a confirmed claim in the child's state store,
//! and the waits that make an extraction request deterministic.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::harness::{self, MockServer, SpawnedChild};
use super::setup::Fixture;

/// The confirmed claim seeded for the recall row. Its object is the fixture
/// table, so the question's `revenue` term selects it at the name tier.
pub(super) const SEEDED_CLAIM: &str = "revenue totals are recorded in dollars";

/// Assisted memory: recall runs, post-turn extraction is permitted, and data
/// sharing is the operator's explicit consent. The budget is `[limits.database]`
/// like part A's, and it arms on a turn's first database use.
fn assisted_config(elapsed_seconds: u64) -> String {
    format!(
        "[ai]\nallow_data_sharing = true\n\n\
         [limits.database]\nelapsed_seconds = {elapsed_seconds}\n\n\
         [memory]\nmode = 'assisted'\n"
    )
}

/// Spawns the REPL with the assisted-memory config. The extra arguments are
/// the same as part A's: the SQLite fixture, the `local` profile, and
/// `bypass` because a piped REPL cannot answer the approval prompt.
pub(super) fn spawn_assisted(
    fixture: &Fixture,
    base_url: &str,
    elapsed_seconds: u64,
    tag: &str,
) -> SpawnedChild {
    let connections = fixture.root.join("connections.toml");
    harness::spawn_child_with_config_extra(
        base_url,
        &assisted_config(elapsed_seconds),
        tag,
        &[
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
            "--approval-mode",
            "bypass",
        ],
    )
}

/// Seeds one confirmed claim through the product's own `contracts remember`
/// command, against the state store the child will open. The child's config
/// root is a pure function of `tag`, so this runs before the child starts.
pub(super) fn seed_claim(fixture: &Fixture, tag: &str, value: &str) {
    let connections = fixture.root.join("connections.toml");
    let output = crate::common::saya_command(
        &harness::config_root_for(tag),
        &[
            "--non-interactive",
            "--format",
            "json",
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
            "contracts",
            "remember",
            "golden.main.revenue",
            "--kind",
            "description",
            "--value",
            value,
        ],
    )
    .output()
    .unwrap();
    assert!(
        output.status.success(),
        "seeding the claim failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Waits until the mock has answered `count` requests. Extraction runs after
/// the answer is already on screen, so waiting on the answer alone would race
/// it and make the golden flaky.
pub(super) fn wait_for_requests(server: &MockServer, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while server.requests.load(Ordering::SeqCst) < count {
        assert!(
            Instant::now() < deadline,
            "the mock did not receive request {count}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The claims the user saw stored: one `memory learned` or `memory noted`
/// line each.
pub(super) fn stored_claims(stdout: &str) -> usize {
    stdout
        .lines()
        .filter(|line| line.contains("memory learned ·") || line.contains("memory noted ·"))
        .count()
}
