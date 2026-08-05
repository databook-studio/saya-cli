/// Case-insensitive fuzzy match. Returns `None` when `query` is NOT a subsequence
/// of `text`; otherwise a score where a HIGHER value is a better match.
///
/// Scoring favours: an exact case-insensitive prefix (largest bonus), matches at
/// word boundaries (start, or right after '_', '.', ' ', '-'), contiguous runs of
/// matched chars, and earlier matches. An empty query returns `Some(0)`.
#[allow(dead_code)]
pub(crate) fn fuzzy_score(text: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }

    let text_chars: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let query_chars: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();

    if query_chars.is_empty() {
        return Some(0);
    }

    let mut score: i32 = 0;
    if text_chars.starts_with(&query_chars) {
        score += 100;
    }

    let mut text_idx = 0;
    let mut last_matched_idx: Option<usize> = None;

    for &q in &query_chars {
        let offset = text_chars[text_idx..].iter().position(|&c| c == q)?;
        let matched_i = text_idx + offset;

        score += 1;

        if matched_i == 0 || matches!(text_chars[matched_i - 1], '_' | '.' | ' ' | '-') {
            score += 5;
        }

        if let Some(prev) = last_matched_idx {
            if matched_i == prev + 1 {
                score += 3;
            } else if matched_i > prev + 1 {
                let gap = (matched_i - prev - 1) as i32;
                score -= gap;
            }
        }

        last_matched_idx = Some(matched_i);
        text_idx = matched_i + 1;
    }

    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuzzy_score() {
        let prefix_score = fuzzy_score("connect", "con");
        let mid_score = fuzzy_score("disconnect", "con");
        assert!(prefix_score.is_some() && mid_score.is_some());
        assert!(prefix_score.unwrap() > mid_score.unwrap());

        assert!(fuzzy_score("connections", "cnt").is_some());
        assert!(fuzzy_score("orders", "xyz").is_none());

        let case_a = fuzzy_score("Orders", "ord");
        let case_b = fuzzy_score("orders", "ORD");
        assert!(case_a.is_some() && case_a == case_b);

        assert_eq!(fuzzy_score("orders", ""), Some(0));

        let boundary_score = fuzzy_score("orders.id", "id");
        let mid_word_score = fuzzy_score("void", "id");
        assert!(boundary_score.is_some() && boundary_score.unwrap() > 0);
        assert!(boundary_score.unwrap() > mid_word_score.unwrap());

        assert!(fuzzy_score("café", "cf").is_some());
    }
}
