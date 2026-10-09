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
