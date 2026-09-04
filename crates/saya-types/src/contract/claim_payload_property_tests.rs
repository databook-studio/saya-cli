//! Property 5 (spec §1): every `ClaimPayload` constructor either returns a
//! payload whose text round-trips the input, or a typed error — never a
//! panic, never a payload containing a control character. Pure. This is the
//! generalisation of the hand-written constructor tests: the constructors are
//! the only thing keeping oversized and control-character-bearing text out of
//! a rendered context block, and a property asks whether that holds across an
//! input space the hand-picked cases did not imagine.
use super::*;
use crate::contract::claim_enums::{Cardinality, ColumnRole};
use crate::{DatabaseObjectKind, ProfileIdentity};
use proptest::prelude::*;

fn make_profile() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
}

fn make_target() -> DatabaseObjectRef {
    let profile = make_profile();
    DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap()
}

/// Any text a user might supply as a description/alias/grain/value: empty,
/// long, control chars, unicode — every rejection path must fire, none panic.
fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(any::<char>(), 0..=1100).prop_map(|chars| chars.into_iter().collect())
}

/// Any column name: same wide alphabet, shorter cap (`MAX_NAME_CHARS`).
fn col_name() -> impl Strategy<Value = String> {
    prop::collection::vec(any::<char>(), 0..=130).prop_map(|chars| chars.into_iter().collect())
}

/// True when `s` contains any Unicode control character.
fn has_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control())
}

/// All five column roles. `select` needs a `Vec`; an array does not satisfy
/// `Into<Cow<'static, [_]>>`.
fn role() -> impl Strategy<Value = ColumnRole> {
    prop::sample::select(vec![
        ColumnRole::Identifier,
        ColumnRole::Dimension,
        ColumnRole::Measure,
        ColumnRole::Timestamp,
        ColumnRole::Sensitive,
    ])
}

/// Every cardinality.
fn cardinality() -> impl Strategy<Value = Cardinality> {
    prop::sample::select(vec![
        Cardinality::OneToOne,
        Cardinality::OneToMany,
        Cardinality::ManyToOne,
        Cardinality::ManyToMany,
    ])
}

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
