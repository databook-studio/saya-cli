//! Scale characterisation for `render` (audit slice H / F08).
//!
//! The optimisation that lands with this file reorders the fold branch so
//! `is_folded` gates range discovery and discovers chapter runs in one pass
//! instead of two whole-slice scans per visited block. Its whole risk is
//! rendered output moving, so these tests pin the output first and ask
//! questions later:
//!
//! * [`render_output_is_unchanged_at_scale`] and
//!   [`a_stale_fold_on_an_evicted_opener_still_unfolds_at_scale`] are
//!   **characterisation** tests: their digests were captured from the
//!   pre-change render on this branch and must reproduce byte-identically
//!   afterwards. They pass both ways by design — tripwires, not evidence
//!   the fix works.
//! * [`one_pass_run_discovery_matches_chapter_range_at_scale`] proves the
//!   one-pass discovery finds the same runs the old whole-slice
//!   `chapter_range` did, and that the bounded foldability `folded_row` now
//!   answers equals the slice-wide `foldable`.
//! * [`range_discovery_comparison_counts`] is the benchmark: a counter of
//!   block-chapter comparisons, deterministic and CI-safe, with the old
//!   algorithm's scan pattern priced arithmetically beside the new code's
//!   measured counter.
//! * [`a_folded_chapter_still_paints_one_summary_row`] is the behaviour the
//!   fold branch exists for, re-asserted at the render level.
//!
//! Digests are FNV-1a over kind, label flag, and text of every rendered row
//! followed by the `starts` navigation vector; lengths are asserted beside
//! each digest so a drift is visible even before the hash diverges.

use std::collections::BTreeSet;

use super::super::MAX_BLOCKS;
use super::super::chapters;
use super::super::rows::WrappedLines;
use super::super::{Block, BlockKind, Transcript};
use super::render::{chapter_run_end, range_probe};

const WIDTH: usize = 80;
const VIEW_HEIGHT: usize = 40;
const SCROLL_UP: usize = 13;

/// A deterministic transcript: one `PRE_CHAPTER` welcome block, then whole
/// chapters of `User` + four responses, with variety (a long answer every
/// 7th chapter, a table block every 11th) so wrapping and table painting are
/// exercised at scale, and a partly-filled live chapter when the budget
/// runs out mid-chapter.
fn fixture(total_blocks: usize) -> Transcript {
    let mut t = Transcript::new();
    t.push(BlockKind::System, "welcome — saya, read-only SQL");
    let mut pushed = 1;
    let mut c = 0u32;
    while pushed < total_blocks {
        c += 1;
        t.push(BlockKind::User, format!("request {c}: count the rows"));
        pushed += 1;
        for a in 1..=4 {
            if pushed >= total_blocks {
                break;
            }
            if a == 3 && c.is_multiple_of(7) {
                t.push(
                    BlockKind::Assistant,
                    format!(
                        "answer {c}.{a} — the counts settle once the bounded query returns, every row accounted for"
                    ),
                );
            } else if a == 4 && c.is_multiple_of(11) {
                t.push(
                    BlockKind::Table,
                    format!("┌ result {c} ──┐\n│ 1 row ok   │\n└────────────┘"),
                );
            } else {
                t.push(BlockKind::Assistant, format!("answer {c}.{a}"));
            }
            pushed += 1;
        }
    }
    t
}

/// FNV-1a 64 over every row (kind, label flag, text bytes, row separator)
/// then every `starts` entry — a digest of the full render result, not a
/// sample of it.
fn digest(rows: &WrappedLines, starts: &[usize]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mix = |byte: u8, h: &mut u64| {
        *h = (*h ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
    };
    for row in rows {
        mix(row.kind as u8, &mut h);
        mix(u8::from(row.is_label), &mut h);
        for b in row.text.as_bytes() {
            mix(*b, &mut h);
        }
        mix(0xFF, &mut h);
    }
    for s in starts {
        for b in s.to_le_bytes() {
            mix(b, &mut h);
        }
    }
    h
}

/// Every finished chapter of a transcript folded through the real API; the
/// folded set and what `folded_row` will honour both fall out of this.
fn fold_finished(t: &mut Transcript) -> BTreeSet<u32> {
    let current = t.blocks().last().map(|b| b.chapter).unwrap_or(0);
    let mut folded = BTreeSet::new();
    for c in 1..current {
        assert!(t.toggle_chapter(c), "chapter {c} must be foldable");
        folded.insert(c);
    }
    folded
}

/// The pre-change render's output at scale, captured on this branch before
/// the one-pass discovery lands (row count + digest per case). The
/// optimisation must reproduce every one of these byte-identically.
const EXPECTED: &[(&str, usize, u64)] = &[
    ("500 unfolded", 1031, 0x5bc3_c098_caba_935e),
    ("500 folded", 108, 0x9381_aafa_e7e9_6a58),
    ("500 scrolled-unfolded", 40, 0x1e74_ccba_3b7c_4640),
    ("500 scrolled-folded", 40, 0x38d8_e457_85e0_fa3a),
    ("2500 unfolded", 5160, 0x9a52_7591_75d2_fb1e),
    ("2500 folded", 508, 0x0442_4249_091b_c8db),
    ("2500 scrolled-unfolded", 40, 0x45fb_ad7e_a928_c72c),
    ("2500 scrolled-folded", 40, 0x55de_3433_2b94_4043),
    ("5000 unfolded", 10321, 0x08b4_e35c_e497_9464),
    ("5000 folded", 1008, 0xa22d_97a3_43ee_ee04),
    ("5000 scrolled-unfolded", 40, 0xe37e_e7c1_9a79_dc99),
    ("5000 scrolled-folded", 40, 0x9499_0d51_add5_87f9),
];

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

/// The one-pass discovery (`chapter_run_end` + the run-start check) must
/// agree with the whole-slice `chapter_range` on every run boundary, and the
/// bounded foldability the range-aware `folded_row` now answers must equal
/// the slice-wide `foldable` — at every scale, including eviction.
#[test]
fn one_pass_run_discovery_matches_chapter_range_at_scale() {
    let check = |blocks: &[Block], label: &str| {
        for i in 0..blocks.len() {
            if i == 0 || blocks[i - 1].chapter != blocks[i].chapter {
                let chapter = blocks[i].chapter;
                let (start, end) = chapters::chapter_range(blocks, chapter)
                    .unwrap_or_else(|| panic!("{label}: chapter {chapter} must have a range"));
                assert_eq!(start, i, "{label}: chapter {chapter} run start");
                assert_eq!(
                    chapter_run_end(blocks, i),
                    end,
                    "{label}: chapter {chapter} run end drifted"
                );
            }
        }
        let current = blocks
            .last()
            .map(|b| b.chapter)
            .unwrap_or(chapters::PRE_CHAPTER);
        let mut seen = BTreeSet::new();
        for b in blocks {
            seen.insert(b.chapter);
        }
        for chapter in seen {
            let bounded = chapter != chapters::PRE_CHAPTER
                && chapter != current
                && blocks
                    .iter()
                    .any(|b| b.chapter == chapter && b.kind == BlockKind::User);
            assert_eq!(
                chapters::foldable(blocks, chapter),
                bounded,
                "{label}: chapter {chapter} foldability drifted"
            );
        }
    };
    for blocks_count in [500, 2_500, 5_000] {
        let t = fixture(blocks_count);
        check(t.blocks(), &format!("{blocks_count}"));
    }
    let evicted = evicted_opener_fixture();
    check(evicted.blocks(), "evicted-opener");
}

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
