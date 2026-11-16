/// One reply the scriptable mock server sends per provider request.
#[cfg(unix)]
#[derive(Clone)]
pub(crate) enum ScriptStep {
    Clarify {
        delay: std::time::Duration,
    },
    Natural,
    /// A natural answer whose usage frame reports `input_tokens` as the
    /// prompt tokens (the figure that becomes the answering input usage).
    NaturalWithUsage {
        input_tokens: u64,
    },
    /// A natural answer with NO usage frame: the numerator stays unknown.
    NaturalNoUsage,
    /// A plain text completion (the summariser's reply shape).
    Summary(String),
    DelayedNatural {
        delay: std::time::Duration,
    },
    Harmless,
    HoldOpen {
        marker: String,
    },
}

#[cfg(unix)]
pub(crate) fn clarification_body() -> String {
    concat!(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,",
        "\"id\":\"call-1\",\"function\":{\"name\":\"request_clarification\",",
        "\"arguments\":\"{\\\"question\\\":\\\"Which one?\\\",\\\"options\\\":[\\\"a\\\",\\\"b\\\"]}\"}}]},",
        "\"finish_reason\":\"tool_calls\"}]}\n\n"
    )
    .to_owned()
}

#[cfg(unix)]
pub(crate) fn answer_body() -> String {
    concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"the-final-answer\"},",
        "\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,",
        "\"completion_tokens\":1}}\n\n"
    )
    .to_owned()
}

#[cfg(unix)]
pub(crate) fn answer_with_usage_body(input_tokens: u64) -> String {
    format!(
        concat!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"the-final-answer\"}},",
            "\"finish_reason\":\"stop\"}}]}}\n\n",
            "data: {{\"choices\":[],\"usage\":{{\"prompt_tokens\":{},",
            "\"completion_tokens\":5}}}}\n\n"
        ),
        input_tokens
    )
}

#[cfg(unix)]
pub(crate) fn answer_no_usage_body() -> String {
    concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"the-final-answer\"},",
        "\"finish_reason\":\"stop\"}]}\n\n"
    )
    .to_owned()
}

#[cfg(unix)]
pub(crate) fn summary_body(text: &str) -> String {
    format!(
        concat!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\"}},",
            "\"finish_reason\":\"stop\"}}]}}\n\n"
        ),
        text = text.replace('"', "'")
    )
}

#[cfg(unix)]
pub(crate) fn harmless_body() -> String {
    "data: {\"content\":\"{\\\"proposals\\\": []}\"}\n\n".to_owned()
}
