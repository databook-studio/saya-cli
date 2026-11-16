//! Totality properties for the single-name constructors: `table_alias` and
//! `default_time_column`.
use super::*;

proptest! {
    /// `table_alias`: a name, validated by `validate_name`.
    #[test]
    fn table_alias_total(s in col_name()) {
        match ClaimPayload::table_alias(&s) {
            Ok(ClaimPayload::TableAlias { alias }) => {
                prop_assert!(!has_control(&alias));
                prop_assert_eq!(&alias, &s);
            }
            Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
            Err(e) => prop_assert!(
                matches!(e,
                    ContractError::EmptyName | ContractError::NameTooLong
                    | ContractError::ControlCharacter),
                "unexpected error: {e:?}"
            ),
        }
    }

    /// `default_time_column`: a name.
    #[test]
    fn default_time_column_total(s in col_name()) {
        match ClaimPayload::default_time_column(&s, None) {
            Ok(ClaimPayload::DefaultTimeColumn { column, .. }) => {
                prop_assert!(!has_control(&column));
                prop_assert_eq!(&column, &s);
            }
            Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
            Err(e) => prop_assert!(
                matches!(e,
                    ContractError::EmptyName | ContractError::NameTooLong
                    | ContractError::ControlCharacter),
                "unexpected error: {e:?}"
            ),
        }
    }
}
