//! Human-readable presentation of native coding results. Pure shaping of the
//! values the application operations return; the operations own the facts.
mod result;
pub(crate) use result::render_coding_result;

#[cfg(test)]
mod result_tests;
