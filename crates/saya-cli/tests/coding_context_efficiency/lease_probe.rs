//! The one job shape that holds a folder's lease, for tests that need a live
//! owner over the folder a run works in.
//!
//! A job that states no write scope only reads, so it takes no lease and keeps
//! nothing another writer can contend with. A write-scoped job does: creating
//! one over a folder another live owner leases is refused, and dropping it
//! gives the lease back. Created and never run, it composes nothing and holds
//! no other handle on the test's state.

use super::fixture::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{CodingAuthorization, CodingError, CodingJobRequest, NativeCodingJob};
use saya_types::JobId;
use std::sync::Arc;

/// A write-scoped in-place job over the fixture's folder, or the refusal the
/// folder's lease answers with.
pub(super) async fn live_writer(
    fixture: &Fixture,
    id: &str,
) -> Result<Arc<NativeCodingJob>, CodingError> {
    let request = CodingJobRequest::new(
        JobId::parse(id).unwrap(),
        "edit this folder".into(),
        vec![".".to_owned()],
    )
    .unwrap();
    let proposed = fixture.application.propose(&request).await.unwrap();
    let approval = CodingAuthorization::scoped(&proposed).unwrap();
    fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
}
