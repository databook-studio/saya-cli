//! Totality property for `join_rule`: the target, the column lists and the
//! condition round-trip, every stored field is control-char-free, and the
//! count/length/name/condition errors are typed.
use super::*;

proptest! {
    /// `join_rule`: the target, the column lists and the condition
    /// round-trip, every stored field is control-char-free, and the
    /// count/length/name/condition errors are typed. Generates up to a few
    /// columns per side; both lists may be empty (a predicate-only join).
    #[test]
    fn join_rule_total(
        target in col_name(),
        locals in prop::collection::vec(col_name(), 0..=4),
        targets in prop::collection::vec(col_name(), 0..=4),
        condition in text(),
    ) {
        match ClaimPayload::join_rule(
            &target,
            locals.clone(),
            targets.clone(),
            &condition,
            None,
        ) {
            Ok(ClaimPayload::JoinRule {
                target: t,
                local_columns,
                target_columns,
                condition: c,
                ..
            }) => {
                for col in local_columns.iter().chain(target_columns.iter()) {
                    prop_assert!(!has_control(col), "control char in join column: {col:?}");
                }
                prop_assert!(!has_control(&t));
                prop_assert!(!has_control(&c));
                prop_assert_eq!(&t, &target);
                prop_assert_eq!(&c, &condition);
                prop_assert_eq!(&local_columns, &locals);
                prop_assert_eq!(&target_columns, &targets);
            }
            Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
            Err(e) => prop_assert!(
                matches!(e,
                    ContractError::EmptyText | ContractError::TextTooLong
                    | ContractError::ControlCharacter
                    | ContractError::EmptyName | ContractError::NameTooLong
                    | ContractError::ColumnCountMismatch | ContractError::TooManyColumns),
                "unexpected error: {e:?}"
            ),
        }
    }
}
