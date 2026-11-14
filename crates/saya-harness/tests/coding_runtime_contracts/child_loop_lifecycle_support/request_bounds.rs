use saya_agent::{ChatMessage, ContextBlock};
use saya_harness::coding::child::{
    ChildExecutionSpec, ChildSpecError, MAX_CHILD_PROFILE_NAMES, MAX_CHILD_REQUEST_BYTES,
    MAX_CHILD_REQUEST_ITEMS, MAX_CHILD_REQUEST_MODEL_CHARS,
};

use super::{authority_view, limits, request};

#[test]
fn child_spec_enforces_serialized_request_and_context_item_bounds() {
    let mut exact = request();
    exact.prompt.clear();
    let empty_prompt_len = serde_json::to_vec(&exact).unwrap().len();
    exact.prompt = "x".repeat(MAX_CHILD_REQUEST_BYTES - empty_prompt_len);
    assert_eq!(
        serde_json::to_vec(&exact).unwrap().len(),
        MAX_CHILD_REQUEST_BYTES
    );
    assert!(
        ChildExecutionSpec::new(
            authority_view("agent-a", "invocation-a"),
            exact.clone(),
            limits(2).0,
            limits(2).1,
            None,
        )
        .is_ok()
    );

    exact.prompt.push('x');
    assert_eq!(
        ChildExecutionSpec::new(
            authority_view("agent-a", "invocation-a"),
            exact,
            limits(2).0,
            limits(2).1,
            None,
        )
        .unwrap_err(),
        ChildSpecError::RequestTooLarge
    );

    let mut max_items = request();
    max_items.history = (0..MAX_CHILD_REQUEST_ITEMS)
        .map(|_| ChatMessage::text("user", "bounded"))
        .collect();
    assert!(
        ChildExecutionSpec::new(
            authority_view("agent-a", "invocation-a"),
            max_items.clone(),
            limits(2).0,
            limits(2).1,
            None,
        )
        .is_ok()
    );

    max_items.context_blocks.push(ContextBlock {
        label: "extra".to_owned(),
        body: "bounded".to_owned(),
        truncated: false,
    });
    assert_eq!(
        ChildExecutionSpec::new(
            authority_view("agent-a", "invocation-a"),
            max_items,
            limits(2).0,
            limits(2).1,
            None,
        )
        .unwrap_err(),
        ChildSpecError::TooManyRequestItems
    );

    let mut exact_model = request();
    exact_model.model = "m".repeat(MAX_CHILD_REQUEST_MODEL_CHARS);
    assert!(
        ChildExecutionSpec::new(
            authority_view("agent-a", "invocation-a"),
            exact_model.clone(),
            limits(2).0,
            limits(2).1,
            None,
        )
        .is_ok()
    );
    exact_model.model.push('m');
    assert_eq!(
        ChildExecutionSpec::new(
            authority_view("agent-a", "invocation-a"),
            exact_model,
            limits(2).0,
            limits(2).1,
            None,
        )
        .unwrap_err(),
        ChildSpecError::ModelNameTooLong
    );

    let mut too_many_profiles = request();
    too_many_profiles.profile_names = (0..=MAX_CHILD_PROFILE_NAMES)
        .map(|index| format!("profile-{index}"))
        .collect();
    assert_eq!(
        ChildExecutionSpec::new(
            authority_view("agent-a", "invocation-a"),
            too_many_profiles,
            limits(2).0,
            limits(2).1,
            None,
        )
        .unwrap_err(),
        ChildSpecError::TooManyProfileNames
    );

    let mut zero_limits = limits(2);
    zero_limits.0.max_turns = Some(0);
    zero_limits.0.max_tool_calls = Some(0);
    zero_limits.0.max_continuations = Some(0);
    zero_limits.1.max_requests = Some(0);
    zero_limits.1.max_tool_calls = Some(0);
    zero_limits.1.max_reported_tokens = Some(0);
    assert!(
        ChildExecutionSpec::new(
            authority_view("agent-a", "invocation-a"),
            request(),
            zero_limits.0,
            zero_limits.1,
            None,
        )
        .is_ok()
    );

    let mut invalid_profiles = request();
    invalid_profiles
        .profile_names
        .push("not a profile".to_owned());
    assert_eq!(
        ChildExecutionSpec::new(
            authority_view("agent-a", "invocation-a"),
            invalid_profiles,
            limits(2).0,
            limits(2).1,
            None,
        )
        .unwrap_err(),
        ChildSpecError::InvalidProfileNames
    );

    let mut unbounded_cases = Vec::with_capacity(5);
    let mut unbounded = limits(2);
    unbounded.0.max_turns = None;
    unbounded_cases.push(unbounded);
    let mut unbounded = limits(2);
    unbounded.0.max_tool_calls = None;
    unbounded_cases.push(unbounded);
    let mut unbounded = limits(2);
    unbounded.0.max_continuations = None;
    unbounded_cases.push(unbounded);
    let mut unbounded = limits(2);
    unbounded.1.max_requests = None;
    unbounded_cases.push(unbounded);
    let mut unbounded = limits(2);
    unbounded.1.max_tool_calls = None;
    unbounded_cases.push(unbounded);
    for (index, (agent, answering)) in unbounded_cases.into_iter().enumerate() {
        assert_eq!(
            ChildExecutionSpec::new(
                authority_view("agent-a", "invocation-a"),
                request(),
                agent,
                answering,
                None,
            )
            .unwrap_err(),
            ChildSpecError::UnboundedLimits,
            "unbounded field case {index}"
        );
    }
}

#[test]
fn uncapped_child_spec_requires_aggregate_limits_to_be_absent() {
    let (agent, answering) = limits(2);
    assert_eq!(
        ChildExecutionSpec::new_uncapped(
            authority_view("agent-a", "invocation-a"),
            request(),
            agent,
            answering,
            None,
        )
        .unwrap_err(),
        ChildSpecError::UnexpectedAggregateLimits
    );
}

#[test]
fn child_spec_separates_initial_request_and_conversation_bounds() {
    for budget in [256 * 1024, 4 * 1024 * 1024] {
        for uncapped in [false, true] {
            let (mut agent, mut answering) = limits(2);
            agent.context_byte_budget = budget;
            if uncapped {
                agent.max_turns = None;
                agent.max_tool_calls = None;
                answering.max_requests = None;
                answering.max_tool_calls = None;
                answering.max_reported_tokens = None;
            }
            let construct = |request| {
                if uncapped {
                    ChildExecutionSpec::new_uncapped(
                        authority_view("agent-a", "invocation-a"),
                        request,
                        agent,
                        answering,
                        None,
                    )
                } else {
                    ChildExecutionSpec::new(
                        authority_view("agent-a", "invocation-a"),
                        request,
                        agent,
                        answering,
                        None,
                    )
                }
            };
            let small = construct(request());
            assert!(
                small.is_ok(),
                "finite conversation budget {budget}: {small:?}"
            );
            let mut exact = request();
            exact.prompt.clear();
            let overhead = serde_json::to_vec(&exact).unwrap().len();
            exact.prompt = "x".repeat(MAX_CHILD_REQUEST_BYTES - overhead);
            assert_eq!(
                serde_json::to_vec(&exact).unwrap().len(),
                MAX_CHILD_REQUEST_BYTES
            );
            assert!(construct(exact.clone()).is_ok());
            exact.prompt.push('x');
            assert_eq!(
                construct(exact).unwrap_err(),
                ChildSpecError::RequestTooLarge
            );
        }
    }
}

#[test]
fn authority_projection_must_fit_the_actual_context_budget() {
    let (mut agent, answering) = limits(2);
    agent.context_byte_budget = 128;
    assert_eq!(
        ChildExecutionSpec::new(
            authority_view("agent-a", "invocation-a"),
            request(),
            agent,
            answering,
            None,
        )
        .unwrap_err(),
        ChildSpecError::RequestTooLarge
    );
}
