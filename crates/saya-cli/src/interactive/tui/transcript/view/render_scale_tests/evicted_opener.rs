/// The pre-change render's output for the stale-fold edge, captured on this
/// branch before the one-pass discovery lands.
const EXPECTED_EVICTED_OPENER: (usize, u64) = (10_000, 0x4887_46a4_296b_c00b);

/// Chapter 1 folded while its opener still survived, then the block bound
/// evicts the opener and mid-chapter survivors remain: the stale-fold edge.
fn evicted_opener_fixture() -> Transcript {
    let mut t = Transcript::new();
    t.push(BlockKind::User, "first request");
    // Chapter 1 gets 301 blocks and the tail 4,948: the bound then evicts
    // 250 blocks — the opener plus 249 tools — leaving mid-chapter
    // survivors behind for the stale-fold edge to be real.
    for i in 0..300 {
        t.push(BlockKind::Tool, format!("early tool {i}"));
    }
    t.push(BlockKind::User, "second request");
    assert!(
        t.toggle_chapter(1),
        "folded while the opener still survived"
    );
    for i in 0..(MAX_BLOCKS - 52) {
        t.push(BlockKind::Tool, format!("tool line {i}"));
    }
    t
}

#[test]
fn a_stale_fold_on_an_evicted_opener_still_unfolds_at_scale() {
    let t = evicted_opener_fixture();
    let (rows, starts) = t.render(WIDTH);
    eprintln!(
        "evicted-opener: blocks={} rows={} hash={:016x}",
        t.blocks().len(),
        rows.len(),
        digest(&rows, &starts)
    );
    assert_eq!(
        (rows.len(), digest(&rows, &starts)),
        EXPECTED_EVICTED_OPENER,
        "the stale-fold render drifted"
    );
    assert_eq!(
        rows.iter().filter(|r| r.text.contains('▸')).count(),
        0,
        "no folded summary may survive an evicted opener"
    );
    assert!(
        t.blocks()[0].chapter == 1 && !t.blocks()[0].text.is_empty(),
        "chapter 1 must keep mid-chapter survivors for this edge to be real"
    );
}
