//! The approval audit's two appends, at a sequence the append itself
//! allocates.
//!
//! A caller that reads the job's tail and appends at `last + 1` takes a number
//! it computed outside the append's transaction: another writer — a live root
//! turn, most of all — can have used that number by the time the append runs.
//! These two entry points take the number inside the same `BEGIN IMMEDIATE`
//! transaction that writes the event, so the number and the row that claims it
//! are decided together, and a failure consumes nothing.

use std::num::NonZeroU64;

use saya_types::{CodingEventV1, JobId};

use super::{
    CodingEventAppend,
    approvals::{begin_immediate, close_request_in_tx, finish, record_request_in_tx},
    events::next_sequence_in_tx,
};
use crate::{SqliteStateStore, StoreError};

impl SqliteStateStore {
    /// Records a job's approval request at the sequence the append allocates.
    /// `build` states the request for that sequence, and may refuse; a refusal
    /// rolls back and leaves the stream untouched.
    pub async fn record_coding_approval_request_at_next_sequence(
        &self,
        job: &JobId,
        build: impl FnOnce(NonZeroU64) -> Result<CodingEventV1, StoreError>,
    ) -> Result<CodingEventAppend, StoreError> {
        let mut transaction = begin_immediate(self).await?;
        let allocated = next_sequence_in_tx(&mut transaction, job).await;
        let outcome = match allocated {
            Ok(sequence) => record_request_in_tx(&mut transaction, build(sequence)).await,
            Err(error) => Err(error),
        };
        finish(transaction, outcome).await
    }

    /// Closes a recorded request at the sequence the append allocates. The
    /// closure sequence is therefore always the one free number, never a
    /// number some other writer took between the question and its answer.
    pub async fn close_coding_approval_at_next_sequence(
        &self,
        job: &JobId,
        build: impl FnOnce(NonZeroU64) -> Result<CodingEventV1, StoreError>,
    ) -> Result<CodingEventAppend, StoreError> {
        let mut transaction = begin_immediate(self).await?;
        let allocated = next_sequence_in_tx(&mut transaction, job).await;
        let outcome = match allocated {
            Ok(sequence) => close_request_in_tx(&mut transaction, build(sequence)).await,
            Err(error) => Err(error),
        };
        finish(transaction, outcome).await
    }
}
