//! The nearest-match hint a zero-match edit carries: up to three regions of
//! the current file most similar to the model's `old_text`, so it can correct
//! its anchor without a fresh read. The hint is bounded by the file's size,
//! the anchor's size, the number of candidates, the lines shown, and the
//! hint's own byte budget — it never becomes a second way to read the file.

use std::collections::HashSet;

/// Hints are skipped entirely above these sizes; similarity work on a larger
/// input costs more than the correction is worth.
const MAX_HINT_FILE_BYTES: usize = 1024 * 1024;
const MAX_HINT_ANCHOR_BYTES: usize = 4 * 1024;
/// The most candidate regions served, and the most lines in one region.
const MAX_CANDIDATES: usize = 3;
const MAX_CANDIDATE_LINES: usize = 8;
/// The hint's whole byte budget and one displayed line's character budget.
const MAX_HINT_BYTES: usize = 2048;
const MAX_LINE_CHARS: usize = 160;

/// The regions of `content` closest to `old_text`, or an empty string when
/// none shares a token with it or the bounds refuse the work.
pub(super) fn nearest_hint(content: &str, old_text: &str) -> String {
    if content.len() > MAX_HINT_FILE_BYTES || old_text.len() > MAX_HINT_ANCHOR_BYTES {
        return String::new();
    }
    let anchor = old_text.to_lowercase();
    let anchor_tokens = tokens(&anchor);
    if anchor_tokens.is_empty() {
        return String::new();
    }
    let original: Vec<&str> = content.lines().collect();
    if original.is_empty() {
        return String::new();
    }
    let lowered = content.to_lowercase();
    let lower: Vec<&str> = lowered.lines().collect();
    let window = old_text.lines().count().clamp(1, MAX_CANDIDATE_LINES);

    let mut best: Vec<Candidate> = Vec::new();
    for start in 0..lower.len() {
        let end = (start + window).min(lower.len());
        let mut seen: Vec<&str> = Vec::new();
        for line in &lower[start..end] {
            for token in line_tokens(line) {
                if !seen.contains(&token) {
                    seen.push(token);
                }
            }
        }
        if seen.is_empty() {
            continue;
        }
        let shared = seen
            .iter()
            .filter(|token| anchor_tokens.contains(*token))
            .count();
        if shared == 0 {
            continue;
        }
        let score = (2 * shared) as f64 / (anchor_tokens.len() + seen.len()) as f64;
        best.push(Candidate { score, start, end });
        best.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then(left.start.cmp(&right.start))
        });
        best.truncate(MAX_CANDIDATES * 2);
    }

    let mut chosen: Vec<Candidate> = Vec::new();
    for candidate in best {
        if chosen.len() == MAX_CANDIDATES {
            break;
        }
        if chosen
            .iter()
            .all(|other| candidate.end <= other.start || candidate.start >= other.end)
        {
            chosen.push(candidate);
        }
    }
    render(&original, &chosen)
}

struct Candidate {
    score: f64,
    start: usize,
    end: usize,
}

fn tokens(text: &str) -> HashSet<&str> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect()
}

fn line_tokens(line: &str) -> impl Iterator<Item = &str> {
    line.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
}

fn render(lines: &[&str], chosen: &[Candidate]) -> String {
    let mut hint = String::new();
    for candidate in chosen {
        let mut block = format!(
            "\nnearest lines {}-{}:\n",
            candidate.start + 1,
            candidate.end
        );
        for line in &lines[candidate.start..candidate.end] {
            block.push_str(&clip(line));
            block.push('\n');
        }
        if hint.len() + block.len() > MAX_HINT_BYTES {
            break;
        }
        hint.push_str(&block);
    }
    hint
}

fn clip(line: &str) -> String {
    if line.chars().count() <= MAX_LINE_CHARS {
        return (*line).to_string();
    }
    let end = line
        .char_indices()
        .nth(MAX_LINE_CHARS)
        .map_or(line.len(), |(index, _)| index);
    format!("{}...", &line[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nearby_region_is_named_with_its_lines() {
        let hint = nearest_hint(
            "one\nlet total = add(a, b);\nthree\n",
            "let total = add(a, c);",
        );
        assert!(hint.contains("nearest lines 2-2:"), "{hint}");
        assert!(hint.contains("let total = add(a, b);"), "{hint}");
    }

    #[test]
    fn a_hint_is_bounded_and_empty_when_nothing_shares_a_token() {
        assert!(nearest_hint("alpha beta\n", "zzz yyy").is_empty());
        let big = "filler line here\n".repeat(80_000);
        assert!(big.len() > MAX_HINT_FILE_BYTES);
        assert!(nearest_hint(&big, "filler").is_empty());
        let hint = nearest_hint(&"filler line here\n".repeat(20_000), "filler");
        assert!(hint.len() <= MAX_HINT_BYTES, "{}", hint.len());
    }
}
