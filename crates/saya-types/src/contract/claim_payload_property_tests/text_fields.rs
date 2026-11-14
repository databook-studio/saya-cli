//! Totality properties for the free-text constructors: `table_description`
//! and `table_grain`'s description.
use super::*;

proptest! {
    /// `table_description`: Ok ⇒ stored text round-trips and has no control
    /// char; Err ⇒ typed error; never panics.
    #[test]
    fn table_description_total(s in text()) {
        match ClaimPayload::table_description(&s) {
            Ok(ClaimPayload::TableDescription { text }) => {
                prop_assert!(!has_control(&text));
                prop_assert_eq!(&text, &s);
            }
            Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
            Err(e) => prop_assert!(
                matches!(e,
                    ContractError::EmptyText | ContractError::TextTooLong
                    | ContractError::ControlCharacter),
                "unexpected error: {e:?}"
            ),
        }
    }

    /// `table_grain`: same totality contract on its `description` field.
    #[test]
    fn table_grain_total(s in text()) {
        match ClaimPayload::table_grain(&s, None) {
            Ok(ClaimPayload::TableGrain { description, .. }) => {
                prop_assert!(!has_control(&description));
                prop_assert_eq!(&description, &s);
            }
            Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
            Err(e) => prop_assert!(
                matches!(e,
                    ContractError::EmptyText | ContractError::TextTooLong
                    | ContractError::ControlCharacter),
                "unexpected error: {e:?}"
            ),
        }
    }
}
