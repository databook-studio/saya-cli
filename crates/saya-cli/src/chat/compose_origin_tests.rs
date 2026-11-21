//! What a session's composed run states about where it was opened from.
//!
//! A chat session and every `saya ask` open a job, so the request a composition
//! creates is a *session* job's request. The test reads it back the way a resume
//! does — from the `request.json` the job wrote — rather than from the
//! composition's own field, so it cannot pass on a value nobody persisted.

use std::sync::Arc;

use saya_agent::ApprovalPolicy;

use super::compose::RunComposition;
use super::compose_fixture::{self, Composed};
use super::operator_front_fixture;
use crate::commands::coding::JobOrigin;
use crate::commands::coding::parent_turn_provider::Scripted;

#[tokio::test]
async fn a_sessions_run_is_a_session_origin_job() {
    let provider = Scripted::start_with_script(Vec::new());
    let fixture = compose_fixture::in_place("origin", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let Composed {
        application,
        run,
        root: _root,
    } = fixture.into_parts(front.front(), "session-origin", ApprovalPolicy::Ask);
    let composition = RunComposition::compose(Arc::clone(&application), run)
        .await
        .expect("the session's run composes");
    let job = composition.job().job_id().clone();
    let resumed = application
        .continuation_request(&job)
        .expect("the session's job resumes the request it was created with");
    assert_eq!(
        resumed.origin(),
        JobOrigin::Session,
        "a session's run is a session job, not a CLI one"
    );
}
