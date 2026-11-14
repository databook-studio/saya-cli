/// The conversation byte budget these tests build against — the same role
/// the loop's `context_byte_budget` plays in production.
const BUDGET: usize = 32 * 1024;

#[path = "tests/budget.rs"]
mod budget;
#[path = "tests/context.rs"]
mod context;
#[path = "tests/selection.rs"]
mod selection;
