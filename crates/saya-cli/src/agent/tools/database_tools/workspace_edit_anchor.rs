//! Anchor matching for the `workspace_edit` replace variant: byte-exact
//! match counting, bounded line-number reporting, and the sha256 digest the
//! refusal and the result carry. Pure helpers — no filesystem contact — so
//! the tool body stays small and these stay unit-testable.

use sha2::{Digest, Sha256};

/// Excerpt budget for an ambiguous-anchor refusal: the error carries bounded
/// line numbers only, never excerpts — this cap bounds how many line numbers
/// are reported.
pub(crate) const WORKSPACE_EDIT_MAX_REPORTED_LINES: usize = 16;

/// Byte offsets of every non-overlapping occurrence of `needle` in `text`.
/// Both are `&str`, so every match boundary is a char boundary and the
/// offsets are valid byte-range ends for the splice. Byte-exact: no
/// normalization, no fuzzy fallback, ever.
pub(crate) fn find_matches(text: &str, needle: &str) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find(needle) {
        let start = from + offset;
        hits.push(start);
        from = start + needle.len();
    }
    hits
}

/// 1-based line numbers for the first few match offsets, bounded so the
/// refusal carries lines — never excerpts, never file content.
pub(crate) fn match_lines(text: &str, hits: &[usize]) -> Vec<u64> {
    let mut starts = vec![0usize];
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    hits.iter()
        .take(WORKSPACE_EDIT_MAX_REPORTED_LINES)
        .map(|hit| starts.partition_point(|start| *start <= *hit) as u64)
        .collect()
}

/// Lowercase hex sha256 of the file's bytes — the digest the refusal and
/// the result carry, and the value an `expected_digest` precondition states.
pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}
