use super::*;

#[test]
fn new_execution_id_is_deterministic_and_charset_bounded() {
    let a = ExecutionEvidence::new_execution_id(1_790_000_000_000, 7);
    let b = ExecutionEvidence::new_execution_id(1_790_000_000_000, 7);
    assert_eq!(a, b);
    assert!(a.starts_with('x'));
    assert!(a.contains('-'));
    assert!(a.len() <= 32);
    let on_charset = |id: &str| {
        id.bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    };
    assert!(on_charset(&a));
    assert_ne!(
        ExecutionEvidence::new_execution_id(1_790_000_000_000, 7),
        ExecutionEvidence::new_execution_id(1_790_000_000_000, 8)
    );
    assert_ne!(
        ExecutionEvidence::new_execution_id(1_790_000_000_000, 7),
        ExecutionEvidence::new_execution_id(1_790_000_001_000, 7)
    );
    let extremes = ExecutionEvidence::new_execution_id(i64::MIN, u64::MAX);
    assert!(extremes.len() <= 32);
    assert!(on_charset(&extremes));
}

#[test]
fn short_id_caps_at_twelve_characters() {
    let mut e = evidence();
    e.execution_id = "xabcdefghijkmnop".to_owned();
    assert_eq!(e.short_id(), "xabcdefghijk");
    e.execution_id = "x1a2b3c-4".to_owned();
    assert_eq!(e.short_id(), "x1a2b3c-4");
}
