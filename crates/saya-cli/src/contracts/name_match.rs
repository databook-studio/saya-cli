//! Matching a prompt term against an object's name segment.
//!
//! Split out of `selection.rs` so the tier logic and the plural rule can be
//! read independently — the rule below is the one place a natural question
//! ("rentals") is reconciled with the table it names (`rental`), and it is
//! deliberately smaller than a stemmer.

/// Whether prompt term `term` (already lowercased, trimmed) matches an object's
/// lowercased name segment at tier 3. A term matches when the segment equals it,
/// equals its singularized form (so a plural prompt term `rentals` names the
/// singular table `rental`), or contains it as a substring (so `orders` still
/// selects `orders0`). Containment is term ⊆ segment, never the reverse: a long
/// term must not match a short table (spec §3 — bidirectional containment is a
/// trap).
///
/// `singular_key` is deliberately small and not a stemmer; see it for the rule
/// and the cases it leaves alone. Irregular plurals (`children`, `people`,
/// `data`) are unsupported by design — naming the limit beats a dependency.
pub(super) fn name_matches(name_segment: &str, term: &str) -> bool {
    if name_segment == term {
        return true;
    }
    let singular = singular_key(term);
    singular != term && name_segment == singular || name_segment.contains(term)
}

/// Whether an alias claim matches the prompt's extracted terms at tier 2.
///
/// An alias is a **phrase** — "account managers", "gross margin", "active
/// customers" — and the terms are single word-ish tokens, because that is what
/// `terms::extract` produces by splitting on non-alphanumerics. Comparing the
/// whole alias to one token by equality, which is what this replaced, meant no
/// multi-word alias could ever match: `"account managers"` is never equal to
/// `"account"` or to `"managers"`. That silently disabled recall for the most
/// valuable kind of fact SAYA can hold, since a single word that already names
/// the table is rarely worth recording in the first place.
///
/// The alias matches when **every** one of its words is present among the
/// prompt's terms, compared with the same plural rule tier 3 uses, so
/// "how many account managers" reaches an alias stored as "account manager".
/// Requiring all words rather than any keeps "account" alone from selecting an
/// object aliased "account managers": a partial phrase is not the phrase.
///
/// An alias whose words are all stop words or shorter than the term floor
/// yields no words to check, and matches nothing — an empty requirement must
/// not be vacuously true, or such an alias would select every object.
pub(super) fn alias_matches(alias: &str, terms: &[String]) -> bool {
    let mut words = alias
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .peekable();
    if words.peek().is_none() {
        return false;
    }
    words.all(|word| {
        terms
            .iter()
            .any(|term| term == word || singular_key(term) == singular_key(word))
    })
}

/// The conservative singular form of `term`, or `term` itself when no rule
/// applies. Rules, in order, for a word ending in `s`:
/// - `ies → y` (`categories → category`), stem ≥ 4 so `series` is left alone;
/// - `ses/xes/zes/ches/shes → drop es` (`addresses → address`, `boxes → box`);
/// - a bare trailing `s → drop`, but not for words ending in `ss` (`address`),
///   `us` (`status`), `is` (`axis`), or `ies` (owned by the rule above).
///
/// Deliberately unsupported: irregular plurals (`children`, `people`, `data`)
/// and anything a real stemmer would catch. A word ending in `s` that is already
/// singular is returned unchanged, so the identity match still holds.
pub(super) fn singular_key(term: &str) -> String {
    let bytes = term.as_bytes();
    let n = bytes.len();
    // `ies → y`, but only when the stem is at least 4 chars so short words like
    // `series` (stem `ser`) are left as-is.
    if n > 3 && term.ends_with("ies") {
        return format!("{}y", &term[..n - 3]);
    }
    // `…ses/xes/zes/ches/shes → drop es`, leaving `s/x/z/ch/sh`.
    if n > 2 && term.ends_with("es") {
        let stem = &term[..n - 2];
        if stem.ends_with(['s', 'x', 'z']) || stem.ends_with("ch") || stem.ends_with("sh") {
            return stem.to_string();
        }
    }
    // Bare trailing `s → drop`, guarded so already-singular `-s` words stay.
    if n > 3
        && term.ends_with('s')
        && !term.ends_with("ss")
        && !term.ends_with("us")
        && !term.ends_with("is")
        && !term.ends_with("ies")
    {
        return term[..n - 1].to_string();
    }
    term.to_string()
}

#[cfg(test)]
mod alias_tests {
    use super::alias_matches;

    fn terms(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_string()).collect()
    }

    /// The regression. `terms::extract` splits on non-alphanumerics, so a
    /// question about "account managers" arrives as two tokens and the old
    /// equality check could never match the stored phrase. Every multi-word
    /// alias — which is most of the ones worth storing — was unrecallable.
    #[test]
    fn a_multi_word_alias_matches_the_words_of_the_question() {
        assert!(alias_matches(
            "account managers",
            &terms(["many", "account", "managers"].as_ref())
        ));
    }

    /// Tier 3's plural rule applies here too: a fact stored in the singular is
    /// still the fact the plural question is about.
    #[test]
    fn plural_and_singular_forms_reach_each_other() {
        assert!(alias_matches(
            "account manager",
            &terms(["account", "managers"].as_ref())
        ));
        assert!(alias_matches(
            "account managers",
            &terms(["account", "manager"].as_ref())
        ));
    }

    /// All words, not any. "account" alone is a different question from
    /// "account managers", and an object aliased the latter must not answer it.
    #[test]
    fn a_partial_phrase_is_not_the_phrase() {
        assert!(!alias_matches(
            "account managers",
            &terms(["account"].as_ref())
        ));
        assert!(!alias_matches("gross margin", &terms(["margin"].as_ref())));
    }

    #[test]
    fn a_single_word_alias_still_matches_exactly_as_before() {
        assert!(alias_matches("reps", &terms(["how", "reps"].as_ref())));
        assert!(!alias_matches("reps", &terms(["staff"].as_ref())));
    }

    /// Word order is not meaningful: the terms are a bag of tokens, deduped and
    /// stop-word filtered, so requiring order would fail on ordinary phrasing.
    #[test]
    fn word_order_does_not_matter() {
        assert!(alias_matches(
            "active customers",
            &terms(["customers", "active"].as_ref())
        ));
    }

    /// An alias that yields no words must match nothing. Vacuous truth here
    /// would select every object in the profile for every question.
    #[test]
    fn an_alias_with_no_words_matches_nothing() {
        assert!(!alias_matches("", &terms(["account"].as_ref())));
        assert!(!alias_matches("   ", &terms(["account"].as_ref())));
        assert!(!alias_matches("--", &terms(["account"].as_ref())));
    }

    #[test]
    fn no_terms_selects_nothing() {
        assert!(!alias_matches("account managers", &[]));
    }
}
