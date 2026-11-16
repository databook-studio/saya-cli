#[test]
fn render_output_is_unchanged_at_scale() {
    let mut actual: Vec<(String, usize, u64)> = Vec::new();
    for blocks in [500, 2_500, 5_000] {
        let (rows, starts) = fixture(blocks).render(WIDTH);
        actual.push((
            format!("{blocks} unfolded"),
            rows.len(),
            digest(&rows, &starts),
        ));

        let mut folded_t = fixture(blocks);
        fold_finished(&mut folded_t);
        let (rows, starts) = folded_t.render(WIDTH);
        actual.push((
            format!("{blocks} folded"),
            rows.len(),
            digest(&rows, &starts),
        ));

        let mut scrolled = fixture(blocks);
        scrolled.scroll_up = SCROLL_UP;
        let view = scrolled.view(WIDTH, VIEW_HEIGHT);
        actual.push((
            format!("{blocks} scrolled-unfolded"),
            view.len(),
            digest(&view, &[]),
        ));

        let mut scrolled_folded = fixture(blocks);
        fold_finished(&mut scrolled_folded);
        scrolled_folded.scroll_up = SCROLL_UP;
        let view = scrolled_folded.view(WIDTH, VIEW_HEIGHT);
        actual.push((
            format!("{blocks} scrolled-folded"),
            view.len(),
            digest(&view, &[]),
        ));
    }
    for (label, rows, hash) in &actual {
        eprintln!("scale {label}: rows={rows} hash={hash:016x}");
    }
    assert_eq!(actual.len(), EXPECTED.len(), "case matrix changed");
    for ((label, rows, hash), (expected_label, expected_rows, expected_hash)) in
        actual.iter().zip(EXPECTED)
    {
        assert_eq!(label, expected_label, "case order changed");
        assert_eq!(*rows, *expected_rows, "{label}: rendered row count drifted");
        assert_eq!(*hash, *expected_hash, "{label}: rendered digest drifted");
    }
}

#[test]
fn a_folded_chapter_still_paints_one_summary_row() {
    let mut t = Transcript::new();
    t.push(BlockKind::User, "count the red orders");
    t.push(BlockKind::Assistant, "the red orders total 42");
    t.push(BlockKind::User, "and the blue ones");
    assert!(t.toggle_chapter(1));
    let (rows, starts) = t.render(WIDTH);
    assert_eq!(rows.len(), 3, "one folded row plus the live label and body");
    assert!(
        rows[0].text.contains("count the red orders"),
        "the folded row carries the verbatim request: {:?}",
        rows[0].text
    );
    assert!(
        rows[0].text.contains("▸"),
        "the folded row names the hidden rows"
    );
    assert!(!rows[0].is_label, "the folded row is one body row");
    // Both chapter-1 blocks carry the summary row's address: navigation can
    // still step to a result inside a fold.
    assert_eq!(starts[0], 0);
    assert_eq!(starts[1], 0);
    assert_eq!(starts[2], 1);
}
