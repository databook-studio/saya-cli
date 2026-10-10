//! The known-secret span finder shared by the runner's one-shot scrub and the
//! command output store's streaming redactor.

/// Returns the merged, sorted, disjoint byte spans of every occurrence of
/// every non-empty secret in `text`, overlapping occurrences included.
/// Strictly overlapping spans are merged; adjacent spans stay separate. Every
/// index is a char boundary of `text`.
pub(crate) fn secret_spans(text: &str, secrets: &[String]) -> Vec<(usize, usize)> {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        let mut from = 0;
        while let Some(found) = text[from..].find(secret.as_str()) {
            let start = from + found;
            spans.push((start, start + secret.len()));
            // Advance one char (not one byte) so overlapping occurrences
            // are found and `from` stays on a char boundary.
            from = start + text[start..].chars().next().map_or(1, char::len_utf8);
        }
    }
    spans.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for (start, end) in spans {
        match merged.last_mut() {
            Some(last) if start < last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}
