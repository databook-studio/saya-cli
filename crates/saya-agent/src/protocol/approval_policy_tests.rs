//! The approval vocabulary's own tests: one spelling per mode, parsed here,
//! unknown values refused — the grammar's authority for every surface.

use super::ApprovalPolicy;
use std::str::FromStr;

/// `bypass` joins the mode vocabulary: it parses, and it is its own name.
/// The unknown-value refusal is the version-skew direction — an older binary
/// falls back to `ask`, never into bypass.
#[test]
fn bypass_joins_the_mode_vocabulary() {
    assert_eq!(
        ApprovalPolicy::from_str("bypass"),
        Ok(ApprovalPolicy::Bypass)
    );
    assert_eq!(ApprovalPolicy::from_str("ask"), Ok(ApprovalPolicy::Ask));
    assert_eq!(
        ApprovalPolicy::from_str("read-only"),
        Ok(ApprovalPolicy::ReadOnly)
    );
    assert_eq!(ApprovalPolicy::from_str("never"), Ok(ApprovalPolicy::Never));
    assert!(
        ApprovalPolicy::from_str("yolo").is_err(),
        "a euphemism is not a spelling: the parse refuses it"
    );
    assert!(
        ApprovalPolicy::from_str("").is_err(),
        "the empty value is not a mode"
    );
    assert!(
        ApprovalPolicy::from_str("auto").is_err(),
        "no softened spelling exists: the mode is bypass, spelled bypass"
    );
}
