//! Totality property for `metric_definition`: the name, definition and
//! columns round-trip, every stored field is control-char-free, and the
//! name/condition/column errors are typed.
use super::*;

proptest! {
    /// `metric_definition`: the name, definition and columns round-trip,
    /// every stored field is control-char-free, and the name/condition/
    /// column errors are typed. The column list may be empty.
    #[test]
    fn metric_definition_total(
        name in col_name(),
        definition in text(),
        columns in prop::collection::vec(col_name(), 0..=4),
    ) {
        match ClaimPayload::metric_definition(&name, &definition, columns.clone(), None) {
            Ok(ClaimPayload::MetricDefinition {
                name: n,
                definition: d,
                columns: cols,
                ..
            }) => {
                for col in cols.iter() {
                    prop_assert!(!has_control(col), "control char in metric column: {col:?}");
                }
                prop_assert!(!has_control(&n));
                prop_assert!(!has_control(&d));
                prop_assert_eq!(&n, &name);
                prop_assert_eq!(&d, &definition);
                prop_assert_eq!(&cols, &columns);
            }
            Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
            Err(e) => prop_assert!(
                matches!(e,
                    ContractError::EmptyText | ContractError::TextTooLong
                    | ContractError::ControlCharacter
                    | ContractError::EmptyName | ContractError::NameTooLong
                    | ContractError::TooManyColumns),
                "unexpected error: {e:?}"
            ),
        }
    }
}
