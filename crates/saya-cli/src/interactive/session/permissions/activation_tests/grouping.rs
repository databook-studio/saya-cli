use super::super::{BYPASS_ON, HOST_LANE_FACT, bypass_line};

/// Property 2 (bypass half): `approvals_are_untouched` — the activation line
/// keeps its bytes with grouping on. The line is emitted through the
/// notice/message paths at launch, `/approvals bypass`, and resume — never
/// through the `AgentEvent` stream the grouper folds — so no group can
/// contain it, split it, or reword it. This pins the emission-site inputs
/// byte-identical: the same facts in, the same line out, whatever the
/// transcript collapsed around it.
#[test]
fn bypass_line_keeps_its_bytes_under_grouping() {
    let staged = vec!["python3".to_string()];
    let first = bypass_line(&staged, false, true, &["curl".to_string()]);
    let second = bypass_line(&staged, false, true, &["curl".to_string()]);
    assert_eq!(
        first, second,
        "the same facts render the same line: grouping has no input"
    );
    assert!(
        first.starts_with(BYPASS_ON),
        "the mode fact opens the line: {first:?}"
    );
    assert!(
        first.contains(HOST_LANE_FACT),
        "the composed lane fact rides along: {first:?}"
    );
    assert!(
        first.contains("denied for this session: curl"),
        "the deny-list segment rides along: {first:?}"
    );
    assert!(
        !first.contains('▸') && !first.contains('▾'),
        "no group marker may touch the activation line: {first:?}"
    );
}
