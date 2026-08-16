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
