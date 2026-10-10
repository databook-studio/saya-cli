//! Durable, payload-free metadata for native coding jobs.

mod records;
pub(crate) mod schema;

pub use records::{
    CodingChildPage, CodingChildRecord, CodingJobRecord, NewCodingChild, NewCodingJob,
};
