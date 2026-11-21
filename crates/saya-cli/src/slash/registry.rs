//! The typo suggestion. [`closest_command`] picks the nearest catalogued name
//! (Levenshtein distance ≤ 2) to suggest in an "unknown command" error; the
//! names come from [`crate::slash::catalog`], so an alias spelling is
//! suggested exactly like the canonical one.

/// Calculates the Levenshtein edit distance between two strings using a single rolling row.
fn levenshtein(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b_chars.len()).collect();

    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, &cb) in b_chars.iter().enumerate() {
            let old_row_j_plus_1 = row[j + 1];
            let cost = if ca == cb { 0 } else { 1 };
            row[j + 1] = (prev + cost).min(row[j] + 1).min(old_row_j_plus_1 + 1);
            prev = old_row_j_plus_1;
        }
    }

    row.last().copied().unwrap_or(0)
}

/// Returns the catalogued name with the smallest Levenshtein distance to
/// `input` if distance <= 2. Aliases take part, so a near-miss on `/quit`
/// still lands on a name that parses.
pub(crate) fn closest_command(input: &str) -> Option<&'static str> {
    let input_lower = input.to_lowercase();
    let mut best_cmd = None;
    let mut min_dist = usize::MAX;

    for cmd in crate::slash::catalog::names() {
        let dist = levenshtein(&input_lower, cmd);
        if dist < min_dist {
            min_dist = dist;
            best_cmd = Some(cmd);
        }
    }

    if min_dist <= 2 { best_cmd } else { None }
}
