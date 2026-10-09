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
