//! Delegation admission for the shared tree: the assignment cap is 32 KiB and
//! the id, dependency, history and batch rules fail closed. The shared fixture
//! and helpers live in `delegate_direct_tests/fixture.rs`; the remaining checks
//! (id rules, dependencies, bounds) live in `delegate_validation_tests.rs`.

#[cfg(unix)]
use super::CodingError;
#[cfg(unix)]
use super::delegate_direct_fixture::{fixture as build, sized_task};

#[cfg(unix)]
#[tokio::test]
async fn direct_mode_accepts_a_32_kib_assignment_and_refuses_32_kib_plus_one() {
    let fixture = build("cap-direct", vec!["only.txt".into()]).await;
    let at_cap = sized_task("child-a", 32 * 1024);
    let over_cap = sized_task("child-b", 32 * 1024 + 1);
    assert!(fixture.job.validate_delegate_tasks(&[at_cap]).is_ok());
    assert!(matches!(
        fixture.job.validate_delegate_tasks(&[over_cap]),
        Err(CodingError::AssignmentTooLong)
    ));
}
