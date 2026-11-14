#[cfg(unix)]
use super::*;

/// Test 1 — the fail-closed chain: probe failure ⇒ the runner is not
/// registered, because no spawn configuration exists, so the tool universe
/// carries no runner tool.
#[cfg(unix)]
#[test]
fn probe_failure_leaves_the_runner_unregistered() {
    let (policy, _root) = policy_with_vanished_root();
    let provision = policy
        .prepare(&runner_program_dir())
        .expect("preparation itself must run");

    // The probe ran and did not prove the runner.
    let report = provision.report();
    assert!(
        !report.proves_runner(),
        "a policy whose roots vanished must not be proven:\n{}",
        report.render()
    );
    assert!(
        !report.failed_required().is_empty(),
        "the report must name what failed:\n{}",
        report.render()
    );

    // The tool list: with no spawn configuration the runner tool cannot join
    // the universe the composition root builds.
    assert!(
        runner_tool_names(&provision).is_empty(),
        "an unproven sandbox registers no runner tool"
    );
    assert!(provision.spawn().is_none());
}
