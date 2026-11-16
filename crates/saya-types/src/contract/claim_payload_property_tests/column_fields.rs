//! Totality properties for column-scoped constructors: `column_description`
//! and `column_role`.
use super::*;

proptest! {
    /// `column_description`: Ok ⇒ both `column` and `text` round-trip and hold
    /// no control char; the `column` is a *name* (MAX_NAME_CHARS) and `text`
    /// is a claim body (MAX_TEXT_CHARS), so the two length bounds are separate.
    #[test]
    fn column_description_total(col in col_name(), body in text()) {
        match ClaimPayload::column_description(&col, &body) {
            Ok(ClaimPayload::ColumnDescription { column, text }) => {
                prop_assert!(!has_control(&column));
                prop_assert!(!has_control(&text));
                prop_assert_eq!(&column, &col);
                prop_assert_eq!(&text, &body);
            }
            Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
            Err(e) => prop_assert!(
                matches!(e,
                    ContractError::EmptyName | ContractError::NameTooLong
                    | ContractError::ControlCharacter
                    | ContractError::EmptyText | ContractError::TextTooLong),
                "unexpected error: {e:?}"
            ),
        }
    }

    /// `column_role`: a name plus a role; Ok ⇒ name round-trips and has no
    /// control char; the role is echoed back by `referenced_columns`.
    #[test]
    fn column_role_total(col in col_name(), r in role()) {
        match ClaimPayload::column_role(&col, r, None) {
            Ok(ClaimPayload::ColumnRole { column, role, .. }) => {
                prop_assert!(!has_control(&column));
                prop_assert_eq!(&column, &col);
                prop_assert_eq!(role, r);
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
