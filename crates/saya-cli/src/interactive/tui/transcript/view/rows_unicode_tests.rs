#[cfg(test)]
mod cell_budget_tests {
    // Audit finding F07: the wrap budget is measured in display cells and
    // every break lands on a grapheme boundary. A `char` is not a cell — `日`
    // is one `char` and two cells, a combining mark is several `char`s and
    // one cell — so a row budgeted by scalar count paints past the frame and
    // the overflow is clipped away for good.
    use super::super::super::BlockKind;
    use super::super::rows::wrap_word_aware;
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";

    fn wrapped(input: &str, width: usize) -> Vec<String> {
        let mut out = Vec::new();
        wrap_word_aware(input, width, BlockKind::System, &mut out);
        out.into_iter().map(|row| row.text).collect()
    }

    /// What a `width`-column frame paints of one row: the row's leading
    /// cells. A row wider than the frame loses its tail — the audit counted
    /// 18 of 36 characters displayed exactly this way.
    fn visible_in_frame(row: &str, width: usize) -> &str {
        let mut end = 0;
        let mut used = 0;
        for grapheme in row.graphemes(true) {
            let w = grapheme.width();
            if used + w > width {
                break;
            }
            used += w;
            end += grapheme.len();
        }
        &row[..end]
    }

    fn visible(rows: &[String], width: usize) -> String {
        rows.iter()
            .map(|row| visible_in_frame(row, width))
            .collect()
    }

    #[test]
    fn cjk_text_is_wrapped_not_clipped() {
        // The audit's reproduction: a 36-character CJK answer in a 20-column
        // frame. Recovery of the whole string from the visible rows is the
        // assertion; counting rows is not.
        let answer: String = (0..36)
            .map(|i| char::from_u32(0x4E00 + i).unwrap())
            .collect();
        let rows = wrapped(&answer, 20);
        assert_eq!(visible(&rows, 20), answer, "rows: {rows:?}");
    }

    #[test]
    fn no_row_exceeds_the_cell_budget() {
        let text = "xxxxxxxx日本語";
        let rows = wrapped(text, 10);
        for row in &rows {
            assert!(
                row.width() <= 10,
                "row {row:?} paints {} cells against a 10-cell budget",
                row.width()
            );
        }
    }

    #[test]
    fn combining_accents_stay_with_their_base() {
        let text = "ab\u{301}cd\u{301}ef";
        let rows = wrapped(text, 2);
        assert_eq!(
            rows,
            vec!["ab\u{301}", "cd\u{301}", "ef"],
            "breaks on grapheme bounds: {rows:?}"
        );
        for row in &rows {
            assert!(
                !row.starts_with('\u{301}'),
                "orphan combining mark leads {row:?}"
            );
        }
    }

    #[test]
    fn an_emoji_sequence_is_never_split() {
        // The space before the family is consumed by the word break (as it
        // is for ASCII), so exact recovery is asserted on the space-free
        // variant; the sequence itself must stay whole wherever it lands.
        let text = format!("hi {FAMILY}!");
        let rows = wrapped(&text, 3);
        assert!(
            rows.iter().any(|row| row.contains(FAMILY)),
            "ZWJ sequence stays whole: {rows:?}"
        );
        for row in &rows {
            assert!(
                !row.starts_with('\u{200D}') && !row.ends_with('\u{200D}'),
                "split inside the sequence: {row:?}"
            );
        }
        let tight = format!("hi{FAMILY}!");
        let tight_rows = wrapped(&tight, 3);
        assert_eq!(tight_rows.concat(), tight, "nothing lost: {tight_rows:?}");
        assert!(tight_rows.iter().any(|row| row.contains(FAMILY)));
    }

    #[test]
    fn ascii_wraps_exactly_as_before() {
        // Control: green before and after the cell-width fix. Pure ASCII has
        // one char == one cell == one grapheme, so the break points are
        // frozen; the transcript snapshots depend on this.
        assert_eq!(
            wrapped("alpha beta gamma", 8),
            vec!["alpha", "beta", "gamma"]
        );
        assert_eq!(
            wrapped("abcdefghij klmno", 6),
            vec!["abcdef", "ghij", "klmno"]
        );
        assert_eq!(wrapped("aaaaaaa bbb", 4), vec!["aaaa", "aaa", "bbb"]);
        assert_eq!(
            wrapped("The quick brown fox jumps over the lazy dog", 12),
            vec!["The quick", "brown fox", "jumps over", "the lazy dog"]
        );
    }

    #[test]
    fn a_single_grapheme_wider_than_the_budget_still_advances() {
        assert_eq!(
            wrapped("\u{65E5}", 1),
            vec!["\u{65E5}"],
            "a 2-cell glyph in a 1-cell budget: emit and advance"
        );
        let text = format!("a{FAMILY}b");
        let rows = wrapped(&text, 1);
        assert_eq!(
            rows,
            vec!["a", FAMILY, "b"],
            "one row per grapheme, no spin, no loss: {rows:?}"
        );
        assert_eq!(rows.concat(), text);
    }
}
