//! Parsing and validation for the headless contracts CLI — slice 2b-2a.
//!
//! This module only parses and validates arguments. It must not touch the store,
//! the schema, or the registry; dispatch and rendering are later slices. Every
//! fallible constructor in `saya_types` is used through `build_payload`, and any
//! `ContractError` it returns is mapped to [`ArgError::InvalidValue`] *without*
//! echoing the offending value — the value is untrusted input the store refuses
//! to persist, and echoing it into a terminal message would undo that refusal.

use thiserror::Error;

use crate::cli::ClaimKindArg;
use saya_types::{ClaimPayload, ColumnRole, KnowledgeSlot};

/// A three-part qualified name `catalog.schema.object`. Never guessed: a one-
/// or two-part name binds the claim to an object the user did not name, and the
/// binding is the whole point of a contract.
#[derive(Debug)]
pub(crate) struct QualifiedName {
    pub catalog: String,
    pub schema: String,
    pub object: String,
}

pub(crate) fn parse_qualified(input: &str) -> Result<QualifiedName, ArgError> {
    let parts: Vec<&str> = input.split('.').map(str::trim).collect();
    if parts.len() != 3 {
        return Err(ArgError::MalformedQualifiedName);
    }
    let [catalog, schema, object] = parts.as_slice() else {
        return Err(ArgError::MalformedQualifiedName);
    };
    if catalog.is_empty() || schema.is_empty() || object.is_empty() {
        return Err(ArgError::MalformedQualifiedName);
    }
    Ok(QualifiedName {
        catalog: catalog.to_string(),
        schema: schema.to_string(),
        object: object.to_string(),
    })
}

/// Maps a kind word to [`ClaimKindArg`]. Accepts clap's canonical kebab-case
/// `--kind` values (the headless form) and snake_case aliases for typing
/// friendliness; both map to the same variant so the agent tool, the slash
/// adapter, and the CLI all agree on what `time-column` means. This is the one
/// kind-word parser in the crate — the headless clap `--kind` derives its
/// vocabulary from `ClaimKindArg`'s `ValueEnum`, and the slash adapter and the
/// `contract_propose` agent tool both call this so no second vocabulary exists.
pub(crate) fn parse_kind(word: &str) -> Option<ClaimKindArg> {
    match word.trim().to_ascii_lowercase().as_str() {
        "description" | "table-description" => Some(ClaimKindArg::Description),
        "alias" | "table-alias" => Some(ClaimKindArg::Alias),
        "grain" | "table-grain" => Some(ClaimKindArg::Grain),
        "time-column" | "time_column" => Some(ClaimKindArg::TimeColumn),
        "column-description" | "column_description" => Some(ClaimKindArg::ColumnDescription),
        "column-role" | "column_role" => Some(ClaimKindArg::ColumnRole),
        _ => None,
    }
}

pub(crate) fn build_payload(
    kind: ClaimKindArg,
    value: &str,
    column: Option<&str>,
    reason: Option<&str>,
) -> Result<ClaimPayload, ArgError> {
    use ClaimKindArg as K;
    // A `ContractError` from a fallible `saya_types` constructor is mapped to
    // `InvalidValue` *without* carrying the offending value — see `ArgError`.
    // We use `.map_err(|_| ArgError::InvalidValue)` rather than `?` so the error
    // never becomes a `ContractError`-carrying variant, and the value stays out.
    //
    // `reason` is forwarded to the directive constructors (`Grain`, `TimeColumn`,
    // `ColumnRole`) only — a reason on a description or alias is not applicable,
    // and the non-directive constructors do not accept one. An empty/whitespace
    // reason collapses to `None` inside the constructor, so a `--reason ""` is
    // the same as no `--reason`.
    match kind {
        K::Description => {
            reject_column(column)?;
            ClaimPayload::table_description(value).map_err(|_| ArgError::InvalidValue)
        }
        K::Alias => {
            reject_column(column)?;
            ClaimPayload::table_alias(value).map_err(|_| ArgError::InvalidValue)
        }
        K::Grain => {
            reject_column(column)?;
            ClaimPayload::table_grain(value, reason).map_err(|_| ArgError::InvalidValue)
        }
        K::TimeColumn => {
            reject_column(column)?;
            ClaimPayload::default_time_column(value, reason).map_err(|_| ArgError::InvalidValue)
        }
        K::ColumnDescription => {
            let column = require_column(column)?;
            ClaimPayload::column_description(column, value).map_err(|_| ArgError::InvalidValue)
        }
        K::ColumnRole => {
            let column = require_column(column)?;
            let role = ColumnRole::parse(value).ok_or(ArgError::UnknownColumnRole)?;
            ClaimPayload::column_role(column, role, reason).map_err(|_| ArgError::InvalidValue)
        }
    }
}

/// The [`KnowledgeSlot`] a `build_payload` payload files under — the one
/// pairing the ingest path and the `remember` write path both use, so a
/// remembered fact lands on the same row `show`/`queue`/recall read (no split
/// brain). Returns `None` for a payload `build_payload` cannot produce (e.g. a
/// `Relationship`), so a caller fails closed rather than guessing a slot.
/// `build_payload` and `slot_for_payload` are the only place the kind→slot
/// pairing lives; the in-crate `contracts/tests.rs` and the integration suites
/// reach for it through here instead of carrying a second copy that could
/// drift.
pub(crate) fn slot_for_payload(payload: &ClaimPayload) -> Option<KnowledgeSlot> {
    match payload {
        ClaimPayload::TableDescription { .. } => Some(KnowledgeSlot::TableDescription),
        ClaimPayload::TableAlias { .. } => Some(KnowledgeSlot::TableAlias),
        ClaimPayload::TableGrain { .. } => Some(KnowledgeSlot::TableGrain),
        ClaimPayload::DefaultTimeColumn { .. } => Some(KnowledgeSlot::TableDefaultTime),
        ClaimPayload::ColumnDescription { column, .. } => Some(KnowledgeSlot::ColumnDescription {
            column: column.clone(),
        }),
        ClaimPayload::ColumnRole { column, .. } => Some(KnowledgeSlot::ColumnRole {
            column: column.clone(),
        }),
        ClaimPayload::JoinRule { .. } => Some(KnowledgeSlot::RelationJoinRule),
        ClaimPayload::MetricDefinition { .. } => Some(KnowledgeSlot::MetricDefinition),
        // `Relationship` is not slot-bound on the `remember` path;
        // `build_payload` never produces it, so a caller that reaches one has a
        // payload it cannot file.
        _ => None,
    }
}

#[derive(Debug)]
pub(crate) enum ReviewDecision {
    Confirm,
    Reject,
}

pub(crate) fn review_decision(confirm: bool, reject: bool) -> Result<ReviewDecision, ArgError> {
    match (confirm, reject) {
        (true, false) => Ok(ReviewDecision::Confirm),
        (false, true) => Ok(ReviewDecision::Reject),
        _ => Err(ArgError::AmbiguousReviewDecision),
    }
}

fn require_column(column: Option<&str>) -> Result<&str, ArgError> {
    column
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .ok_or(ArgError::ColumnRequired)
}

fn reject_column(column: Option<&str>) -> Result<(), ArgError> {
    if let Some(c) = column
        && !c.trim().is_empty()
    {
        return Err(ArgError::ColumnNotApplicable);
    }
    Ok(())
}

/// Errors from contracts argument parsing. Payload-free: a bad `--kind` is
/// caught by clap before this code runs, so no variant carries the offending
/// value. Echoing untrusted input into a terminal message is exactly what the
/// store refuses to persist it for.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub(crate) enum ArgError {
    #[error("qualified name must be exactly three dot-separated parts: catalog.schema.object")]
    MalformedQualifiedName,
    #[error("this claim kind does not accept a column")]
    ColumnNotApplicable,
    #[error("this claim kind requires a column")]
    ColumnRequired,
    #[error("unknown column role")]
    UnknownColumnRole,
    #[error("claim value is invalid")]
    InvalidValue,
    #[error("choose exactly one of --confirm or --reject")]
    AmbiguousReviewDecision,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ClaimKindArg as K;
    use saya_types::ClaimPayload;

    /// Sentinel that must never appear in any rendered [`ArgError`] message.
    const SENTINEL: &str = "SENTINELVALUE";

    #[test]
    fn parse_kind_canonical_and_aliases() {
        // Kebab (headless clap) and snake_case aliases map to the same variant.
        assert_eq!(parse_kind("alias"), Some(K::Alias));
        assert_eq!(parse_kind("table-alias"), Some(K::Alias));
        assert_eq!(parse_kind("time-column"), Some(K::TimeColumn));
        assert_eq!(parse_kind("time_column"), Some(K::TimeColumn));
        assert_eq!(parse_kind("column-role"), Some(K::ColumnRole));
        assert_eq!(parse_kind("column_role"), Some(K::ColumnRole));
        assert_eq!(parse_kind("description"), Some(K::Description));
        assert_eq!(parse_kind("grain"), Some(K::Grain));
        assert_eq!(parse_kind("column-description"), Some(K::ColumnDescription));
    }

    #[test]
    fn parse_kind_trims_and_lowercases() {
        assert_eq!(parse_kind("  Alias  "), Some(K::Alias));
        assert_eq!(parse_kind("ALIAS"), Some(K::Alias));
    }

    #[test]
    fn parse_kind_unknown_is_none() {
        assert_eq!(parse_kind("not-a-kind"), None);
        assert_eq!(parse_kind(""), None);
    }

    #[test]
    fn parse_qualified_accepts_three_parts() {
        let q = parse_qualified("analytics.public.orders").unwrap();
        assert_eq!(q.catalog, "analytics");
        assert_eq!(q.schema, "public");
        assert_eq!(q.object, "orders");
    }

    #[test]
    fn parse_qualified_rejects_wrong_arity() {
        assert!(parse_qualified("orders").is_err());
        assert!(parse_qualified("public.orders").is_err());
        assert!(parse_qualified("a.b.c.d").is_err());
    }

    #[test]
    fn parse_qualified_rejects_empty_part() {
        assert!(parse_qualified("a..c").is_err());
        assert!(parse_qualified("  .b.c").is_err());
    }

    #[test]
    fn parse_qualified_trims_whitespace_around_parts() {
        let spaced = parse_qualified(" analytics . public . orders ").unwrap();
        let plain = parse_qualified("analytics.public.orders").unwrap();
        assert_eq!(spaced.catalog, plain.catalog);
        assert_eq!(spaced.schema, plain.schema);
        assert_eq!(spaced.object, plain.object);
    }

    #[test]
    fn build_payload_description() {
        let p = build_payload(K::Description, "orders fact table", None, None).unwrap();
        assert!(matches!(p, ClaimPayload::TableDescription { .. }));
    }

    #[test]
    fn build_payload_alias() {
        let p = build_payload(K::Alias, "orders", None, None).unwrap();
        assert!(matches!(p, ClaimPayload::TableAlias { .. }));
    }

    #[test]
    fn build_payload_grain() {
        let p = build_payload(K::Grain, "one row per order", None, None).unwrap();
        assert!(matches!(p, ClaimPayload::TableGrain { .. }));
    }

    /// A `--reason` on a directive kind is carried onto the payload (the point
    /// of `claim-reasons`: the explicit-remember path can state *why*). A reason
    /// on a non-directive kind (description/alias) is ignored — only the
    /// directive constructors accept one.
    #[test]
    fn build_payload_carries_reason_on_directive_kinds() {
        let grain = build_payload(
            K::Grain,
            "one row per order",
            None,
            Some("orders ship separately"),
        )
        .unwrap();
        assert!(matches!(
            grain,
            ClaimPayload::TableGrain { reason: Some(r), .. } if r == "orders ship separately"
        ));
        let time = build_payload(
            K::TimeColumn,
            "return_date",
            None,
            Some("a rental only counts once it comes back"),
        )
        .unwrap();
        assert!(matches!(
            time,
            ClaimPayload::DefaultTimeColumn { reason: Some(r), .. }
            if r == "a rental only counts once it comes back"
        ));
        let role =
            build_payload(K::ColumnRole, "measure", Some("amount"), Some("money paid")).unwrap();
        assert!(matches!(
            role,
            ClaimPayload::ColumnRole { reason: Some(r), .. } if r == "money paid"
        ));
        // A reason on a non-directive kind is not forwarded (the description
        // constructor takes none); it does not error, it is simply not carried.
        let desc = build_payload(K::Description, "a table", None, Some("ignored")).unwrap();
        assert!(matches!(desc, ClaimPayload::TableDescription { .. }));
    }

    #[test]
    fn build_payload_column_description() {
        let p = build_payload(K::ColumnDescription, "order total", Some("amount"), None).unwrap();
        assert!(matches!(p, ClaimPayload::ColumnDescription { .. }));
    }

    #[test]
    fn build_payload_column_role() {
        let p = build_payload(K::ColumnRole, "measure", Some("amount"), None).unwrap();
        assert!(matches!(p, ClaimPayload::ColumnRole { .. }));
    }

    #[test]
    fn column_kinds_require_column() {
        assert_eq!(
            build_payload(K::ColumnDescription, "x", None, None).unwrap_err(),
            ArgError::ColumnRequired
        );
        assert_eq!(
            build_payload(K::ColumnRole, "measure", None, None).unwrap_err(),
            ArgError::ColumnRequired
        );
    }

    #[test]
    fn table_kinds_reject_column() {
        for kind in [K::Description, K::Alias, K::Grain, K::TimeColumn] {
            assert_eq!(
                build_payload(kind, "x", Some("amount"), None).unwrap_err(),
                ArgError::ColumnNotApplicable
            );
        }
    }

    #[test]
    fn unknown_column_role() {
        assert_eq!(
            build_payload(K::ColumnRole, "not-a-role", Some("amount"), None).unwrap_err(),
            ArgError::UnknownColumnRole
        );
    }

    #[test]
    fn invalid_value_control_character() {
        let err = build_payload(
            K::Description,
            &format!("hello{SENTINEL}\nworld"),
            None,
            None,
        )
        .unwrap_err();
        assert_eq!(err, ArgError::InvalidValue);
    }

    #[test]
    fn invalid_value_too_long() {
        let long = format!("{SENTINEL}{}", "x".repeat(2000));
        let err = build_payload(K::Description, &long, None, None).unwrap_err();
        assert_eq!(err, ArgError::InvalidValue);
    }

    #[test]
    fn arg_error_display_never_echoes_value() {
        // Each reachable ArgError is built from input containing SENTINEL and
        // must omit it from its rendered message.
        let invalid =
            build_payload(K::Description, &format!("{SENTINEL}\n"), None, None).unwrap_err();
        assert_eq!(invalid, ArgError::InvalidValue);
        assert!(!format!("{invalid}").contains(SENTINEL));

        let not_applicable = build_payload(K::Description, "ok", Some(SENTINEL), None).unwrap_err();
        assert_eq!(not_applicable, ArgError::ColumnNotApplicable);
        assert!(!format!("{not_applicable}").contains(SENTINEL));

        let required = build_payload(K::ColumnRole, SENTINEL, None, None).unwrap_err();
        assert_eq!(required, ArgError::ColumnRequired);
        assert!(!format!("{required}").contains(SENTINEL));

        let unknown = build_payload(K::ColumnRole, SENTINEL, Some("amount"), None).unwrap_err();
        assert_eq!(unknown, ArgError::UnknownColumnRole);
        assert!(!format!("{unknown}").contains(SENTINEL));

        let malformed = parse_qualified(SENTINEL).unwrap_err();
        assert_eq!(malformed, ArgError::MalformedQualifiedName);
        assert!(!format!("{malformed}").contains(SENTINEL));

        let ambiguous = review_decision(true, true).unwrap_err();
        assert_eq!(ambiguous, ArgError::AmbiguousReviewDecision);
        assert!(!format!("{ambiguous}").contains(SENTINEL));
    }

    #[test]
    fn review_decision_confirm_only() {
        assert!(matches!(
            review_decision(true, false).unwrap(),
            ReviewDecision::Confirm
        ));
    }

    #[test]
    fn review_decision_reject_only() {
        assert!(matches!(
            review_decision(false, true).unwrap(),
            ReviewDecision::Reject
        ));
    }

    #[test]
    fn review_decision_ambiguous() {
        assert_eq!(
            review_decision(false, false).unwrap_err(),
            ArgError::AmbiguousReviewDecision
        );
        assert_eq!(
            review_decision(true, true).unwrap_err(),
            ArgError::AmbiguousReviewDecision
        );
    }
}

#[cfg(test)]
mod property_tests {
    //! Properties 8 & 9 (spec §3): qualified-name parsing is total and lossless,
    //! and the kind vocabulary agrees across the CLI (`clap` `ValueEnum`), the
    //! slash adapter, and the file parser — all three reach `parse_kind` for the
    //! latter two, and this property ties `clap`'s canonical name to the same
    //! variant. Pure: no store, no filesystem, no async.
    use super::{parse_kind, parse_qualified};
    use crate::cli::ClaimKindArg;
    use clap::ValueEnum;
    use proptest::prelude::*;

    /// A clean qualified-name component: non-empty, control-char-free, dot-free,
    /// and with no leading/trailing whitespace so `trim` is the identity — the
    /// round-trip is then exact, not the trimmed form.
    fn component() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_é-]{1,8}"
    }

    /// Any string at all, including control chars, dots, unicode, empty — for the
    /// totality / count property. Nothing may panic.
    fn any_string() -> impl Strategy<Value = String> {
        prop::collection::vec(any::<char>(), 0..=40).prop_map(|chars| chars.into_iter().collect())
    }

    /// Independent oracle for the acceptance condition: exactly three
    /// dot-separated parts, each non-empty after trim.
    fn accepts_three(s: &str) -> bool {
        let parts: Vec<&str> = s.split('.').map(str::trim).collect();
        parts.len() == 3 && parts.iter().all(|p| !p.is_empty())
    }

    proptest! {
        /// Property 8a — lossless round-trip: three clean components joined by `.`
        /// parse back to the same three components via the getters, and the
        /// qualified name is the canonical `catalog.schema.object` form.
        #[test]
        fn parse_qualified_round_trips_three_components(
            catalog in component(),
            schema in component(),
            object in component(),
        ) {
            let input = format!("{catalog}.{schema}.{object}");
            let q = parse_qualified(&input).expect("three clean components must parse");
            prop_assert_eq!(&q.catalog, &catalog);
            prop_assert_eq!(&q.schema, &schema);
            prop_assert_eq!(&q.object, &object);
            // The qualified name is the canonical join; build it from the getters
            // after the equality asserts have consumed nothing (they borrow).
            let joined = format!("{}.{}.{}", q.catalog, q.schema, q.object);
            prop_assert_eq!(&joined, &input);
        }

        /// Property 8b — totality and count agreement: for any string,
        /// `parse_qualified` is Ok exactly when the independent oracle says the
        /// shape is three non-empty trimmed parts, and it never panics.
        #[test]
        fn parse_qualified_total_and_count_agrees(s in any_string()) {
            let result = std::panic::catch_unwind(|| parse_qualified(&s));
            prop_assert!(result.is_ok(), "parse_qualified panicked on: {s:?}");
            let expected = accepts_three(&s);
            let parsed = result.unwrap();
            prop_assert_eq!(parsed.is_ok(), expected);
        }
    }

    /// Property 9 — kind vocabulary agreement. The CLI derives its `--kind`
    /// vocabulary from `ClaimKindArg`'s `ValueEnum`; the slash adapter and the
    /// file parser both route through `parse_kind`. Every word `clap` accepts for
    /// a variant (its canonical name and any aliases) must therefore parse via
    /// `parse_kind` to that same variant — otherwise the three surfaces disagree,
    /// which is exactly the drift the shared parser exists to prevent.
    #[test]
    fn kind_vocabulary_agrees_across_cli_slash_and_file() {
        for variant in ClaimKindArg::value_variants() {
            let possible = variant
                .to_possible_value()
                .expect("every ClaimKindArg variant has a possible value");
            // The canonical name and every alias clap accepts for this variant.
            for word in possible.get_name_and_aliases() {
                assert_eq!(
                    parse_kind(word),
                    Some(*variant),
                    "clap word {word:?} for {variant:?} does not parse_kind to the same variant"
                );
            }
        }
    }
}
