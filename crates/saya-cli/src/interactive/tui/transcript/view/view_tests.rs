#[cfg(test)]
mod label_row_red_tests {
    // RED: these tests name the `Row` API (`row.text`, `row.is_label`) that
    // does not exist yet — `WrappedLines` is still `Vec<(BlockKind, String)>`,
    // so this module fails to compile until `rows.rs` lands.
    use super::super::super::rows::WrappedLines;
    use super::super::super::{BlockKind, Transcript};

    fn texts(rows: &WrappedLines) -> Vec<&str> {
        rows.iter().map(|row| row.text.as_str()).collect()
    }

    #[test]
    fn every_non_empty_block_gains_exactly_one_label_row() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "hello");
        t.push(BlockKind::Assistant, "hi");
        let rows = t.wrapped(80);
        assert_eq!(rows.len(), 4, "two bodies plus two labels: {rows:?}");
        assert!(rows[0].is_label && rows[0].text == "YOU");
        assert!(!rows[1].is_label && rows[1].text == "hello");
        assert!(rows[2].is_label && rows[2].text == "SAYA");
        assert!(!rows[3].is_label && rows[3].text == "hi");
    }

    #[test]
    fn an_empty_block_gains_no_label_row() {
        let mut t = Transcript::new();
        t.push(BlockKind::System, "");
        let rows = t.wrapped(80);
        assert_eq!(rows.len(), 1, "a spacer keeps its bare empty row");
        assert!(
            rows.iter().all(|row| !row.is_label),
            "but gains no label row: {rows:?}"
        );
    }

    #[test]
    fn find_never_lands_on_a_label_row() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "nothing to match here");
        assert_eq!(
            t.count_matches("saya", 80),
            0,
            "the only 'saya' is the SAYA label row"
        );
    }

    #[test]
    fn consecutive_same_kind_blocks_each_get_their_own_label() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "first");
        t.push(BlockKind::User, "second");
        let rows = t.wrapped(80);
        let labels: Vec<&str> = rows
            .iter()
            .filter(|row| row.is_label)
            .map(|row| row.text.as_str())
            .collect();
        assert_eq!(labels, vec!["YOU", "YOU"]);
    }

    #[test]
    fn total_lines_still_equals_the_rows_produced() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "hello");
        t.push(BlockKind::Assistant, "hi");
        let rows = t.wrapped(80);
        assert_eq!(t.total_lines(80), texts(&rows).len());
        assert_eq!(t.total_lines(80), 4);
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::rows::WrappedLines;
    use super::super::super::{BlockKind, Transcript};
    use super::super::super::{MAX_BLOCKS, MAX_TOTAL_TEXT_BYTES};
    use std::rc::Rc;

    #[test]
    fn test_transcript_all() {
        let mut t = Transcript::new();
        assert!(t.is_empty() && t.blocks().is_empty());
        t.push(BlockKind::User, "Hello");
        t.push(BlockKind::Assistant, "Hi there");
        t.append_delta(BlockKind::Assistant, "!");
        t.append_delta(BlockKind::User, "Bye");
        assert_eq!(t.blocks().len(), 3);
        assert_eq!(t.blocks()[1].text, "Hi there!");

        t.push(BlockKind::Assistant, "streaming answer");
        t.reformat_last(BlockKind::Assistant, |s| format!("[formatted: {s}]"));
        assert_eq!(t.blocks()[3].text, "[formatted: streaming answer]");

        let mut t2 = Transcript::new();
        t2.push(BlockKind::System, "1234567890abcdefghij");
        let s: String = t2.wrapped(10).iter().map(|row| row.text.as_str()).collect();
        assert_eq!(s, "1234567890abcdefghij");

        let (mut t3, mut t4) = (Transcript::new(), Transcript::new());
        t3.push(BlockKind::Error, "a\n\nb\n");
        t4.push(BlockKind::Tool, "日日日日日");
        // `日` paints two terminal cells, so a 2-cell budget fits one glyph
        // per row: five body rows under the ACTIVITY label.
        assert!(t3.wrapped(10).len() == 4 && t4.wrapped(2).len() == 6);

        t.clear();
        t.push(BlockKind::User, "l1\nl2\nl3\nl4\nl5");
        let texts = |v: WrappedLines| v.into_iter().map(|row| row.text).collect::<Vec<_>>();
        assert_eq!(texts(t.view(10, 3)), ["l3", "l4", "l5"]);
        t.scroll_up(1, 10, 3);
        assert_eq!(texts(t.view(10, 3)), ["l2", "l3", "l4"]);
        t.scroll_up(100, 10, 3);
        assert_eq!(texts(t.view(10, 3)), ["YOU", "l1", "l2"]);
        t.scroll_down(1);
        assert_eq!(t.view(10, 3)[0].text, "l1");
        t.scroll_down(10);
        assert_eq!(t.view(10, 3)[0].text, "l3");
        t.scroll_up(2, 10, 3);
        t.scroll_to_bottom();
        assert_eq!(t.view(10, 3)[0].text, "l3");

        let (mut empty, mut t_edge) = (Transcript::new(), Transcript::new());
        empty.scroll_up(5, 10, 5);
        empty.scroll_down(2);
        empty.scroll_to_bottom();
        t_edge.push(BlockKind::User, "hello");
        assert!(empty.view(10, 5).is_empty());
        assert_eq!(empty.total_lines(10), 0);
        assert!(t_edge.view(10, 0).is_empty());

        let mut tc = Transcript::new();
        tc.push(BlockKind::User, "hello world");
        let (l1, l2) = (tc.lines(10), tc.lines(10));
        assert!(Rc::ptr_eq(&l1, &l2) && tc.wrapped(10).len() == 3);

        tc.push(BlockKind::Assistant, "hi");
        let (l3, l4) = (tc.lines(10), tc.lines(20));
        assert!(!Rc::ptr_eq(&l2, &l3) && !Rc::ptr_eq(&l3, &l4));
        tc.append_delta(BlockKind::Assistant, " there");
        assert!(!Rc::ptr_eq(&l4, &tc.lines(20)));
        tc.clear();
        assert!(tc.lines(20).is_empty());

        let mut tb = Transcript::new();
        (0..MAX_BLOCKS + 100).for_each(|i| tb.push(BlockKind::User, format!("msg {i}")));
        assert_eq!(tb.blocks().len(), MAX_BLOCKS);
        assert_eq!(tb.blocks()[0].text, "msg 100");
        let last_text = format!("msg {}", MAX_BLOCKS + 99);
        assert_eq!(tb.blocks()[MAX_BLOCKS - 1].text, last_text);

        let mut tb2 = Transcript::new();
        tb2.push(BlockKind::User, "old block");
        let huge = "x".repeat(MAX_TOTAL_TEXT_BYTES);
        tb2.push(BlockKind::Assistant, huge.clone());
        assert_eq!(tb2.blocks().len(), 1);
        assert_eq!(tb2.blocks()[0].text, huge);
        assert_eq!(tb2.view(10, 1)[0].kind, BlockKind::Assistant);
    }

    #[test]
    fn test_transcript_max_blocks_large_input_bound() {
        let mut t = Transcript::new();
        let total_pushed = MAX_BLOCKS + 5000;
        for i in 0..total_pushed {
            t.push(BlockKind::User, format!("block {i}"));
        }

        assert!(
            t.blocks().len() <= MAX_BLOCKS,
            "blocks length must be clamped to <= MAX_BLOCKS"
        );
        assert_eq!(t.blocks().len(), MAX_BLOCKS);

        let expected_first = format!("block {}", total_pushed - MAX_BLOCKS);
        let expected_last = format!("block {}", total_pushed - 1);

        assert_eq!(t.blocks().first().unwrap().text, expected_first);
        assert_eq!(t.blocks().last().unwrap().text, expected_last);

        let view_lines = t.view(80, 20);
        assert_eq!(
            view_lines.len(),
            20,
            "view(width, height) must return exactly height lines after stress"
        );

        let wrapped = t.wrapped(80);
        assert_eq!(
            wrapped.len(),
            t.total_lines(80),
            "wrapped length must match total_lines length"
        );
        assert_eq!(wrapped.len(), MAX_BLOCKS * 2);
    }

    #[test]
    fn test_transcript_total_bytes_eviction() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "initial early message");

        let chunk_size = 1_500_000;
        let large_1 = "a".repeat(chunk_size);
        let large_2 = "b".repeat(chunk_size);
        let large_3 = "c".repeat(chunk_size);

        t.push(BlockKind::Assistant, large_1);
        t.push(BlockKind::Assistant, large_2);
        t.push(BlockKind::Assistant, large_3.clone());

        let total_bytes: usize = t.blocks().iter().map(|b| b.text.len()).sum();
        assert!(
            total_bytes <= MAX_TOTAL_TEXT_BYTES,
            "Total text bytes ({total_bytes}) must be <= MAX_TOTAL_TEXT_BYTES ({MAX_TOTAL_TEXT_BYTES})"
        );

        assert_ne!(
            t.blocks().first().unwrap().text,
            "initial early message",
            "Initial early message must be evicted"
        );
        assert_eq!(
            t.blocks().last().unwrap().text,
            large_3,
            "Newest pushed block must be retained"
        );
    }
}
