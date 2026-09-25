//! The folded summary's fit (re-audit R01): its budget is `width` terminal
//! cells, so marker, prefix, and the request itself are measured in cells
//! and cut on grapheme boundaries — a wide request paints within `width`,
//! and a cut never lands between a base and its combining mark.

use super::super::{BlockKind, Transcript};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// A folded chapter whose request is wide: the one-row summary must fit its
/// `width` cells. The row paints as-is (the view clips horizontally), so an
/// overrun is real overflow, not a wrap.
#[test]
fn a_folded_cjk_summary_fits_its_width() {
    let mut t = Transcript::new();
    t.push(BlockKind::User, "日本".repeat(40));
    t.push(BlockKind::Assistant, "the answer");
    t.push(BlockKind::User, "next");
    assert!(t.toggle_chapter(1));
    let rows = t.wrapped(80);
    assert_eq!(
        rows.len(),
        3,
        "one folded row plus the live chapter: {rows:?}"
    );
    let row = &rows[0];
    assert!(
        row.text.contains("…"),
        "precondition: the wide request must truncate with an ellipsis: {:?}",
        row.text
    );
    assert!(
        row.text.width() <= 80,
        "the folded summary must fit its 80-cell width ({} cells): {:?}",
        row.text.width(),
        row.text
    );
}

/// A char-counted budget cuts `e` + U+0301 (one grapheme, two chars) between
/// base and mark: every kept cluster must be whole.
#[test]
fn a_folded_summary_never_splits_a_grapheme() {
    let mut t = Transcript::new();
    t.push(BlockKind::User, "e\u{0301}".repeat(100));
    t.push(BlockKind::Assistant, "the answer");
    t.push(BlockKind::User, "next");
    assert!(t.toggle_chapter(1));
    let rows = t.wrapped(81);
    assert!(
        rows[0].text.contains("…"),
        "precondition: the request must be cut at this width: {:?}",
        rows[0].text
    );
    let body = rows[0]
        .text
        .strip_prefix("YOU  ")
        .expect("the folded row still leads with the role word");
    let kept = body.split('…').next().expect("the split always yields");
    assert!(
        kept.graphemes(true).all(|g| g == "e\u{0301}"),
        "every kept cluster must be a whole base+mark grapheme: {kept:?}"
    );
}
