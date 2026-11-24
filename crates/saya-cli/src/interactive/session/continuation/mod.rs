//! Runtime-only, bounded facts for the next request in an interactive session.

mod assemble;
pub(crate) mod compaction;
mod outcomes;
mod record;
mod record_block;
mod restore;
mod summary;
pub(crate) use assemble::{assemble_continuation, digest_resolver};
pub(crate) use outcomes::PriorToolOutcomes;
pub(crate) use record::{SessionObservations, now_unix_ms, source_digest};
pub(crate) use record_block::render_record_block;
pub(crate) use restore::restore_continuation;
pub(crate) use summary::render_summary_block;

pub(in crate::interactive) fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
