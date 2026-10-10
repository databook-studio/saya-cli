//! Durable, payload-free metadata for native coding jobs.

mod events;
mod projection;
mod records;
mod replay;
pub(crate) mod schema;

pub use events::{CodingEventAppend, MAX_CODING_EVENT_PAGE_SIZE};
pub use records::{
    CodingChildPage, CodingChildRecord, CodingJobRecord, NewCodingChild, NewCodingJob,
};
pub use replay::CodingEventPage;
