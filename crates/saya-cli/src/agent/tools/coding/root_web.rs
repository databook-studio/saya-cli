//! The direct-mode root's web lane: one bounded GET of an https page, and one
//! bounded query of the configured search backend — each put to the operator
//! under `ask`.
//!
//! The root advertises these tools whenever the job's approval mode reaches the
//! network at all and the job's own egress could be built. Which of the two
//! exists is [`crate::agent::tools::web::WebTools::compose`]'s answer, and it
//! is the same answer every surface gets: `web_fetch` wherever the mode reaches
//! a host, `web_search` only where the user also configured a backend.
//!
//! It follows the root shell's placement, and for the same reason: the root's
//! egressing tools come with its working checkout. A job with no write path
//! composes the coordinator's delegation surface alone, which reaches no host.

use std::sync::Arc;

use crate::agent::tools::consent::Caller;
use crate::agent::tools::web::WebTools;
use crate::commands::coding::NativeCodingJob;

/// Builds the root's web lane, or `None` where this job's mode advertises none,
/// or where no egress could be composed at all.
pub(crate) fn build(job: &NativeCodingJob) -> Option<Arc<WebTools>> {
    let caller = Caller::Root {
        job_id: job.job_id().clone(),
    };
    let egress = job.web_egress()?;
    // The operator, when one is present, is told which job is asking. Under
    // `ask` with none, the tools still exist — and refuse every call with the
    // reason naming the flag that would reach the network.
    WebTools::compose(
        &egress,
        job.approval_policy(),
        job.operator(),
        &caller,
        job.cancellation(),
    )
    .map(Arc::new)
}
