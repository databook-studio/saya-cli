//! What a coordinator reads of one durable child completion: its identity and outcome and, for a
//! failed child, why it failed. The view remains for tests.

#[cfg(all(test, unix))]
#[path = "completion_view_tests.rs"]
mod tests;
