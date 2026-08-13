//! Prompt term extraction for recall — slice 2b-3b §2.
//!
//! Deterministic and pure: no store, no schema, no clock. The prompt is
//! untrusted input driving a scan, so the output is bounded (explicit refs and
//! terms both capped) before it reaches [`crate::contracts::recall`].
//!
//! Two kinds of signal come out of a free-text prompt:
//! - **Explicit references** like `@catalog.schema.table`, parsed with
//!   [`super::args::parse_qualified`] on the text after `@`. A malformed one is
//!   skipped silently — the user is writing prose, not a command.
//! - **Terms**: lowercase word-ish tokens, deduplicated, length- and count-bounded.

use super::args::{QualifiedName, parse_qualified};

/// The signal extracted from a prompt: explicit `@catalog.schema.table` refs
/// plus the word-ish terms selection matches against.
pub(crate) struct PromptTerms {
    pub explicit: Vec<QualifiedName>,
    pub terms: Vec<String>,
}

/// Minimum and maximum length for a term to be kept. The cap matters: the
/// prompt is untrusted input driving a store scan.
const MIN_TERM_LEN: usize = 3;
const MAX_TERM_LEN: usize = 64;
/// Hard cap on the number of terms carried into recall.
const MAX_TERMS: usize = 32;
/// Hard cap on explicit refs; a prompt with dozens of `@ref`s is not a query.
const MAX_EXPLICIT: usize = 32;

/// Stop words dropped from the term list before the cap. Kept small and
/// common; selection still matches on the substantive tokens that remain.
const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "from", "with", "show", "what", "how", "many", "all", "me", "by", "in",
    "of", "to", "is", "are",
];

/// Extracts the recall signal from `prompt`. Pure: same input → same output.
pub(crate) fn extract(prompt: &str) -> PromptTerms {
    PromptTerms {
        explicit: explicit_refs(prompt),
        terms: terms(prompt),
    }
}

/// Every `@catalog.schema.table` in the prompt, in first-seen order, deduped,
/// capped at [`MAX_EXPLICIT`]. A malformed `@…` is skipped silently.
fn explicit_refs(prompt: &str) -> Vec<QualifiedName> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for candidate in prompt.split('@').skip(1) {
        // The text after `@` runs to the next whitespace — a qualified name is
        // a single token. Anything longer is prose, not a reference.
        let token = candidate.split_whitespace().next().unwrap_or("");
        if token.is_empty() {
            continue;
        }
        let Ok(q) = parse_qualified(token) else {
            continue; // malformed — the user is writing prose, not a command
        };
        let key = format!("{}.{}.{}", q.catalog, q.schema, q.object);
        if seen.insert(key) {
            out.push(q);
            if out.len() >= MAX_EXPLICIT {
                break;
            }
        }
    }
    out
}

/// Lowercase word-ish tokens, split on non-alphanumeric-and-underscore, deduped
/// (first-seen order), each 3–64 chars, stop words dropped, capped at 32.
fn terms(prompt: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for token in prompt.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        let lower = token.to_ascii_lowercase();
        if lower.len() < MIN_TERM_LEN || lower.len() > MAX_TERM_LEN {
            continue;
        }
        if STOP_WORDS.contains(&lower.as_str()) {
            continue;
        }
        if seen.insert(lower.clone()) {
            out.push(lower);
            if out.len() >= MAX_TERMS {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_stop_words() {
        let t = extract("show me all the orders by month for the user");
        // "show","me","all","the","by","for","the" are stop words; remaining
        // substantive tokens are kept in first-seen order.
        assert_eq!(t.terms, vec!["orders", "month", "user"]);
    }

    #[test]
    fn dedups_terms_case_insensitively() {
        let t = extract("Orders orders ORDERS by month Month");
        assert_eq!(t.terms, vec!["orders", "month"]);
    }

    #[test]
    fn caps_at_32_terms() {
        // 33 distinct words, each well within bounds — the 33rd is dropped.
        let words: Vec<String> = (0..33).map(|i| format!("word{i:02}")).collect();
        let prompt = words.join(" ");
        let t = extract(&prompt);
        assert_eq!(t.terms.len(), 32);
        assert_eq!(t.terms.last(), Some(&"word31".to_string()));
        assert!(!t.terms.contains(&"word32".to_string()));
    }

    #[test]
    fn drops_too_short_and_too_long_tokens() {
        let two = "ab"; // 2 chars — dropped
        let sixty_five = "a".repeat(65); // 65 chars — dropped
        let three = "abc"; // 3 chars — kept
        let sixty_four = "b".repeat(64); // 64 chars — kept
        let prompt = format!("{two} {sixty_five} {three} {sixty_four}");
        let t = extract(&prompt);
        assert!(t.terms.contains(&"abc".to_string()));
        assert!(t.terms.contains(&"b".repeat(64)));
        assert!(!t.terms.contains(&"ab".to_string()));
        assert!(!t.terms.contains(&"a".repeat(65)));
    }

    #[test]
    fn malformed_explicit_ref_is_skipped_without_error() {
        // `@orders` is not three-part — skipped. `@analytics.public.orders` is.
        let t = extract("look at @orders and @analytics.public.orders please");
        assert_eq!(t.explicit.len(), 1);
        assert_eq!(t.explicit[0].object, "orders");
        assert_eq!(t.explicit[0].schema, "public");
        assert_eq!(t.explicit[0].catalog, "analytics");
    }

    #[test]
    fn dedups_explicit_refs() {
        let t = extract("@analytics.public.orders @analytics.public.orders");
        assert_eq!(t.explicit.len(), 1);
    }

    #[test]
    fn caps_explicit_refs() {
        let mut prompt = String::new();
        for i in 0..40 {
            prompt.push_str(&format!("@c.s.t{i} "));
        }
        let t = extract(&prompt);
        assert_eq!(t.explicit.len(), 32);
    }

    #[test]
    fn empty_prompt_yields_no_signal() {
        let t = extract("   \n\t  ");
        assert!(t.explicit.is_empty());
        assert!(t.terms.is_empty());
    }

    #[test]
    fn split_on_non_alphanumeric_and_underscore() {
        let t = extract("orders/monthly!report#summary");
        assert_eq!(t.terms, vec!["orders", "monthly", "report", "summary"]);
    }
}
