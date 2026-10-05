//! Fitting the projected instruction blocks to the request's remaining
//! bytes. The preflight (`fit_request`) evicts history only — it never trims
//! block bodies — so the projection fits its own blocks here, with the
//! accounting the request build enforces: [`turn_bytes`], the exact
//! post-escape, post-wrapper size of the user turn. Later sources are fitted
//! first, mirroring the loader's personal-then-project order: the earlier
//! source is the more stable prefix, so the later one yields first.

use super::{
    InstructionProjection, MIN_BLOCK_BODY_BYTES, OmittedReason, OmittedSource, SourceReceipt,
    UNTRUSTED_PREFIX,
};
use saya_agent::{ContextBlock, turn_bytes};
use saya_harness::instructions::{InstructionOrigin, SkippedSource};

/// The user-turn labels for the two instruction origins.
fn label_of(origin: InstructionOrigin) -> &'static str {
    match origin {
        InstructionOrigin::Personal => "personal-instructions",
        InstructionOrigin::Project => "project-instructions (AGENTS.md)",
    }
}

/// One admitted source prepared for the request fit: the redacted body text
/// plus the whole-file identity the receipt carries.
pub(super) struct Admitted {
    pub(super) origin: InstructionOrigin,
    pub(super) file_name: &'static str,
    pub(super) text: String,
    pub(super) source_truncated: bool,
    pub(super) sha256: String,
    pub(super) file_size: u64,
    pub(super) redactions: usize,
}

/// One block body: the fixed untrusted framing line, a blank line, then the
/// redacted source text truncated to `text_bytes`.
fn body(text: &str, text_bytes: usize) -> String {
    format!("{UNTRUSTED_PREFIX}\n\n{}", &text[..text_bytes])
}

/// The exact post-escape byte cost these blocks add to the user turn,
/// measured with [`turn_bytes`] against an empty-turn baseline.
fn blocks_cost(blocks: &[ContextBlock]) -> usize {
    turn_bytes(None, blocks, "").saturating_sub(turn_bytes(None, &[], ""))
}

/// The largest byte length of `text` at or below `upper` that is a UTF-8 boundary.
fn floor_boundary(text: &str, upper: usize) -> usize {
    let mut end = upper.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// Fits the admitted sources into `remaining` request bytes, later source
/// first. Earlier sources are measured at full length with the worst-case
/// truncation wrapper, so a block that fits its search fits the final
/// request; drops record a [`OmittedReason::NoBudget`] omission.
pub(super) fn fit_to_request_bytes(
    admitted: Vec<Admitted>,
    remaining: usize,
    skipped: Vec<SkippedSource>,
) -> InstructionProjection {
    let mut suffix: Vec<(ContextBlock, SourceReceipt)> = Vec::new();
    let mut omitted_rev: Vec<OmittedSource> = Vec::new();
    for index in (0..admitted.len()).rev() {
        let source = &admitted[index];
        let fits = |text_bytes: usize| -> bool {
            let mut blocks: Vec<ContextBlock> = admitted[..index]
                .iter()
                .map(|earlier| ContextBlock {
                    label: label_of(earlier.origin).to_string(),
                    body: body(&earlier.text, earlier.text.len()),
                    truncated: true,
                })
                .collect();
            blocks.push(ContextBlock {
                label: label_of(source.origin).to_string(),
                body: body(&source.text, text_bytes),
                truncated: true,
            });
            blocks.extend(suffix.iter().map(|(block, _)| block.clone()));
            blocks_cost(&blocks) <= remaining
        };
        let full = source.text.len();
        let Some(kept) = keep_bytes(source, full, fits) else {
            omitted_rev.push(OmittedSource {
                origin: source.origin,
                file_name: source.file_name,
                reason: OmittedReason::NoBudget,
            });
            continue;
        };
        let truncated = source.source_truncated || kept < full;
        let block = ContextBlock {
            label: label_of(source.origin).to_string(),
            body: body(&source.text, kept),
            truncated,
        };
        suffix.push((
            block,
            SourceReceipt {
                origin: source.origin,
                file_name: source.file_name,
                sha256: source.sha256.clone(),
                file_size: source.file_size,
                included_bytes: kept,
                truncated,
                redactions: source.redactions,
            },
        ));
    }
    suffix.reverse();
    omitted_rev.reverse();
    let (blocks, sources) = suffix.into_iter().unzip();
    InstructionProjection {
        blocks,
        sources,
        skipped,
        omitted: omitted_rev,
    }
}

/// The byte length of the source text the block keeps, or `None` when even
/// the framing line does not fit or the fit would leave a stub below
/// [`MIN_BLOCK_BODY_BYTES`].
fn keep_bytes(source: &Admitted, full: usize, fits: impl Fn(usize) -> bool) -> Option<usize> {
    if fits(full) {
        return Some(full);
    }
    if !fits(0) {
        return None;
    }
    let text = &source.text;
    let mut lo = 0usize;
    let mut hi = full;
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(floor_boundary(text, mid)) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let kept = floor_boundary(text, lo);
    (body(text, kept).len() >= MIN_BLOCK_BODY_BYTES).then_some(kept)
}
