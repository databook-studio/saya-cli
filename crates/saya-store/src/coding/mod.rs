//! Durable, payload-free metadata for native coding jobs.

mod budget_records;
mod budgets;
mod checkout_event;
mod checkout_operations;
mod checkout_reads;
mod checkouts;
mod events;
mod projection;
mod records;
mod replay;
pub(crate) mod schema;

pub use budget_records::{CodingBudgetReservationPage, CodingBudgetReservationRecord};
pub use checkout_operations::MAX_CHECKOUT_OPERATION_INTENTS_PER_JOB;
pub use checkout_reads::CheckoutOperationPage;
pub use checkouts::{MAX_CHECKOUT_RECORDS_PER_JOB, ManagedCheckoutPage, ManagedCheckoutRecord};
pub use events::{CodingEventAppend, MAX_CODING_EVENT_PAGE_SIZE};
pub use records::{
    CodingChildPage, CodingChildRecord, CodingJobRecord, NewCodingChild, NewCodingJob,
};
pub use replay::CodingEventPage;
