/// Prices the pre-change fold branch: for every visited block it ran
/// `chapter_range` unconditionally (left-to-right `&&`, before `is_folded`
/// could veto), whose `position()` compares from the front to the run start
/// (`start + 1`) and whose `rposition()` compares from the back to the run
/// end (`len - end + 1`). Runs are swept once and priced arithmetically, so
/// pricing a whole session costs O(blocks) — it measures the old algorithm's
/// exact scan pattern without paying it.
fn old_render_price(blocks: &[Block], folded: &BTreeSet<u32>) -> usize {
    let mut runs: Vec<(u32, usize, usize)> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        match runs.last_mut() {
            Some((chapter, _, end)) if *chapter == b.chapter => *end = i + 1,
            _ => runs.push((b.chapter, i, i + 1)),
        }
    }
    let current = blocks
        .last()
        .map(|b| b.chapter)
        .unwrap_or(chapters::PRE_CHAPTER);
    let mut price = 0;
    let mut skip_until = 0;
    for (i, block) in blocks.iter().enumerate() {
        if i < skip_until {
            continue;
        }
        let (start, end) = runs
            .iter()
            .find(|&&(chapter, _, _)| chapter == block.chapter)
            .map(|&(_, start, end)| (start, end))
            .unwrap();
        price += start + 1 + (blocks.len() - end + 1);
        let foldable = block.chapter != chapters::PRE_CHAPTER
            && block.chapter != current
            && blocks[start..end].iter().any(|b| b.kind == BlockKind::User);
        if start == i && folded.contains(&block.chapter) && foldable {
            skip_until = end;
        }
    }
    price
}

const SCROLLED_APPENDS: usize = 50;

/// One tail render of a finished transcript: what one paint pays.
fn tail_counts(blocks_count: usize, folded: bool) -> (usize, usize) {
    let mut t = fixture(blocks_count);
    let folded_set = if folded {
        fold_finished(&mut t)
    } else {
        BTreeSet::new()
    };
    range_probe::reset();
    t.total_lines(WIDTH);
    (
        range_probe::take(),
        old_render_price(t.blocks(), &folded_set),
    )
}

/// A scrolled streaming session: the last `SCROLLED_APPENDS` blocks land
/// while the reader is scrolled up, so every append re-renders through the
/// compensation path (`push.rs`'s `after_scrolled_append`). Counts the
/// discovery comparisons across all of those renders — the repeated cost the
/// audit names — against the old algorithm's price for the same session.
fn scrolled_counts(blocks_count: usize, folded: bool) -> (usize, usize) {
    let prefix = blocks_count - SCROLLED_APPENDS;
    let mut t = fixture(prefix);
    let folded_set = if folded {
        fold_finished(&mut t)
    } else {
        BTreeSet::new()
    };
    t.scroll_up = 1;
    t.total_lines(WIDTH); // warm the cache so each append compensates
    range_probe::reset();
    let mut c = t.blocks().last().map(|b| b.chapter).unwrap_or(0);
    for j in 0..(SCROLLED_APPENDS / 5) {
        c += 1;
        t.push(BlockKind::User, format!("request {c}: tail {j}"));
        for a in 1..=4 {
            t.push(BlockKind::Assistant, format!("answer {c}.tail{a}"));
        }
    }
    let new_count = range_probe::take();
    // The final blocks sliced at every append length is exactly the state
    // each compensating render saw (appends never evict below the bound).
    let final_blocks: Vec<Block> = t.blocks().to_vec();
    let mut old_total = 0;
    for n in (prefix + 1)..=final_blocks.len() {
        old_total += old_render_price(&final_blocks[..n], &folded_set);
    }
    (new_count, old_total)
}

/// The deliverable measurement: block-chapter comparisons spent on range
/// discovery, per configuration, at each scale — printed as a table and
/// gated on the complexity change (new < old everywhere; unfolded free;
/// folded linear; old superlinear).
#[test]
fn range_discovery_comparison_counts() {
    let mut table: Vec<(String, usize, usize, usize, usize)> = Vec::new();
    for blocks_count in [500, 2_500, 5_000] {
        for folded in [false, true] {
            let fold = if folded { "folded" } else { "unfolded" };
            let (new_tail, old_tail) = tail_counts(blocks_count, folded);
            let (new_scrolled, old_scrolled) = scrolled_counts(blocks_count, folded);
            eprintln!(
                "range-discovery {blocks_count} tail/{fold}: renders=1 old={old_tail} new={new_tail}"
            );
            eprintln!(
                "range-discovery {blocks_count} scrolled/{fold}: renders={SCROLLED_APPENDS} old={old_scrolled} new={new_scrolled}"
            );
            table.push((
                format!("{blocks_count} tail/{fold}"),
                blocks_count,
                1,
                old_tail,
                new_tail,
            ));
            table.push((
                format!("{blocks_count} scrolled/{fold}"),
                blocks_count,
                SCROLLED_APPENDS,
                old_scrolled,
                new_scrolled,
            ));
        }
    }
    for (label, _, _, old_count, new_count) in &table {
        assert!(
            new_count < old_count,
            "{label}: discovery must get cheaper (old={old_count} new={new_count})"
        );
    }
    for (label, _, _, _, new_count) in table.iter().filter(|(l, ..)| l.contains("unfolded")) {
        assert_eq!(*new_count, 0, "{label}: unfolded discovery must be free");
    }
    let cell = |label: &str| table.iter().find(|(l, ..)| l == label).unwrap();
    // Old work grows quadratically in block count; folded discovery stays
    // linear (every folded run pays its own length once, plus one check).
    assert!(cell("2500 tail/unfolded").3 > 4 * cell("500 tail/unfolded").3);
    assert!(cell("5000 tail/unfolded").3 > 4 * cell("2500 tail/unfolded").3);
    assert!(
        cell("5000 tail/folded").4 <= 2 * 5_000,
        "folded discovery must stay linear"
    );
}
