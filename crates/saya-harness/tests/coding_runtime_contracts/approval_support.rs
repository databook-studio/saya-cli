//! Explicit immediate host consent for pure authority fixtures. This does not
//! assert checkout freshness; public ownership/process tests establish that.
use async_trait::async_trait;
use saya_agent::{ApprovalDecider, ToolDefinition};
use saya_harness::coding::policy::{AuthorityError, RootDelegationInput, RootDelegationScope};
use std::{
    future::Future,
    task::{Context, Poll},
};
struct FixtureHost;
#[async_trait]
impl ApprovalDecider for FixtureHost {
    async fn approve(&self, tool: &ToolDefinition, _: &serde_json::Value) -> bool {
        tool.name == "coding_root_scope"
    }
}
pub fn approved_root(input: RootDelegationInput) -> Result<RootDelegationScope, AuthorityError> {
    let mut approval = Box::pin(RootDelegationScope::approve(input, &FixtureHost));
    // The fixture decider has no waits. Poll once rather than entering a nested
    // runtime inside synchronous grammar helpers used by async owner tests.
    match approval
        .as_mut()
        .poll(&mut Context::from_waker(&futures_util::task::noop_waker()))
    {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("immediate fixture consent unexpectedly waited"),
    }
}
