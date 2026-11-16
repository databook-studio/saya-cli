//! The stop class a failed job is shown with in the human-readable job view.

use super::*;

fn show(value: &serde_json::Value) -> String {
    render_coding_result(&CodingCommand::Show { id: "job-1".into() }, value)
}

fn job(failure: serde_json::Value) -> serde_json::Value {
    json!({"job_id":"job-1","state":"failed","failure":failure,"next_cursor":null,
        "children":[{"agent_id":"impl","state":"failed","invocation_id":"inv-1"}]})
}

#[test]
fn show_names_the_class_and_the_child_a_failed_job_stopped_with() {
    let text = show(&job(
        json!({"event":{"type":"run_terminated","state":"failed",
        "failure":{"class":"provider_stream_ended","agent_id":"impl","invocation_id":"inv-1"}}}),
    ));
    assert!(
        text.contains("Stopped: provider_stream_ended (agent impl)"),
        "{text}"
    );
    assert!(text.contains("Job job-1: failed, 1 child agent"), "{text}");
}

#[test]
fn show_names_a_class_that_no_child_caused() {
    let text = show(&job(
        json!({"event":{"type":"run_terminated","state":"failed",
        "failure":{"class":"coordinator_stalled","agent_id":null,"invocation_id":null}}}),
    ));
    assert!(text.contains("Stopped: coordinator_stalled\n"), "{text}");
}

#[test]
fn show_of_a_job_with_no_recorded_stop_says_nothing_about_one() {
    for failure in [json!(null), json!({"event":{"type":"state_changed"}})] {
        let text = show(&job(failure));
        assert!(!text.contains("Stopped"), "{text}");
        assert!(text.contains("Job job-1: failed"), "{text}");
    }
}
