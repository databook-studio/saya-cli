//! Totality property for `relationship`: the two column lists and the
//! cardinality round-trip, every stored column is control-char-free, and the
//! count/length errors are typed.
use super::*;

proptest! {
    /// `relationship`: the two column lists and the cardinality round-trip,
    /// every stored column is control-char-free, and the count/length errors
    /// are typed. Generates up to a few columns per side.
    #[test]
    fn relationship_total(
        locals in prop::collection::vec(col_name(), 0..=4),
        targets in prop::collection::vec(col_name(), 0..=4),
        card in cardinality(),
    ) {
        let target = make_target();
        match ClaimPayload::relationship(target, locals.clone(), targets.clone(), card) {
            Ok(ClaimPayload::Relationship {
                local_columns,
                target_columns,
                cardinality,
                ..
            }) => {
                for c in local_columns.iter().chain(target_columns.iter()) {
                    prop_assert!(!has_control(c), "control char in relationship column: {c:?}");
                }
                prop_assert_eq!(&local_columns, &locals);
                prop_assert_eq!(&target_columns, &targets);
                prop_assert_eq!(cardinality, card);
            }
            Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
            Err(e) => prop_assert!(
                matches!(e,
                    ContractError::EmptyColumns | ContractError::ColumnCountMismatch
                    | ContractError::TooManyColumns
                    | ContractError::EmptyName | ContractError::NameTooLong
                    | ContractError::ControlCharacter),
                "unexpected error: {e:?}"
            ),
        }
    }
}
