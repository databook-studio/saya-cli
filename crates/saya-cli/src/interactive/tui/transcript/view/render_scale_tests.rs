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

include!("render_scale_tests/render_scale.rs");

include!("render_scale_tests/evicted_opener.rs");
include!("render_scale_tests/range_discovery.rs");

include!("render_scale_tests/range_scale.rs");
