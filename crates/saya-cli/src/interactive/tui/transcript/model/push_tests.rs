#[cfg(test)]
mod viewport_freeze_tests {
    use crate::interactive::tui::transcript::MAX_BLOCKS;
    use crate::interactive::tui::transcript::rows::Row;
    use crate::interactive::tui::transcript::{BlockKind, Transcript};

    fn texts(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|row| row.text.clone()).collect()
    }

    fn view_texts(t: &Transcript, width: usize, height: usize) -> Vec<String> {
        texts(&t.view(width, height))
    }

    #[test]
    fn scrolled_up_push_keeps_the_same_top_row_visible() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "alpha");
        t.push(BlockKind::User, "beta");
        t.push(BlockKind::Assistant, "gamma");
        t.scroll_up(1, 80, 4);
        let before = view_texts(&t, 80, 4);
        assert_eq!(before[0], "alpha");
        t.push(BlockKind::Assistant, "delta");
        assert_eq!(
            view_texts(&t, 80, 4),
            before,
            "appending while scrolled up must not move the visible rows"
        );
    }

    #[test]
    fn following_the_tail_still_shows_new_lines() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "alpha");
        t.push(BlockKind::Assistant, "gamma");
        assert!(t.is_following_tail());
        t.push(BlockKind::Assistant, "brand new");
        let shown = view_texts(&t, 80, 4);
        assert!(
            shown.iter().any(|line| line == "brand new"),
            "the live tail must still show new lines: {shown:?}"
        );
    }

    #[test]
    fn a_streaming_delta_does_not_move_a_scrolled_reader() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "question one");
        t.push(BlockKind::User, "question two");
        t.push(BlockKind::Assistant, "begin");
        t.scroll_up(2, 80, 4);
        let top = t.view(80, 4)[0].text.clone();
        for i in 0..10 {
            t.append_delta(BlockKind::Assistant, "\nmore");
            assert_eq!(
                t.view(80, 4)[0].text,
                top,
                "streaming chunk {i} moved the scrolled reader"
            );
        }
    }

    #[test]
    fn eviction_does_not_run_the_anchor_past_the_start() {
        let mut t = Transcript::new();
        for i in 0..MAX_BLOCKS {
            t.push(BlockKind::User, format!("m{i}"));
        }
        t.scroll_up(50, 80, 20);
        for i in 0..100 {
            t.push(BlockKind::User, format!("n{i}"));
        }
        let total = t.total_lines(80);
        assert!(
            t.scroll_up <= total,
            "the anchor must stay bounded by the content: {} > {total}",
            t.scroll_up
        );
        let (measured, start) = t.scroll_metrics(80, 20);
        assert_eq!(measured, total);
        assert!(
            start + 20 <= total,
            "the metrics window must sit inside the content"
        );
        assert_eq!(t.view(80, 20).len(), 20);
    }

    #[test]
    fn a_resize_does_not_count_as_new_activity() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "some question here");
        let body = (1..=10)
            .map(|i| format!("answer line number {i} with enough words to wrap"))
            .collect::<Vec<_>>()
            .join("\n");
        t.push(BlockKind::Assistant, body);
        t.scroll_up(2, 80, 4);
        assert!(t.scroll_up > 0, "precondition: the reader has scrolled up");
        let anchor = t.scroll_up;
        let view = t.view(80, 4);
        let top = view[0].text.clone();
        let _ = t.view(20, 4);
        let _ = t.total_lines(20);
        assert_eq!(
            t.scroll_up, anchor,
            "re-wrapping at a new width is not an append"
        );
        assert_eq!(t.view(80, 4)[0].text, top);
    }

    #[test]
    fn the_scrollbar_still_points_at_the_visible_window() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "alpha");
        t.push(BlockKind::User, "beta");
        t.push(BlockKind::Assistant, "gamma");
        t.scroll_up(1, 80, 4);
        for step in 0..3 {
            t.push(BlockKind::Assistant, format!("extra {step}"));
            let wrapped = t.wrapped(80);
            let (total, start) = t.scroll_metrics(80, 4);
            assert_eq!(
                total,
                wrapped.len(),
                "measured height must equal painted height"
            );
            assert_eq!(
                view_texts(&t, 80, 4),
                texts(&wrapped[start..start + 4]),
                "the scrollbar window must be the visible window"
            );
        }
    }
}
