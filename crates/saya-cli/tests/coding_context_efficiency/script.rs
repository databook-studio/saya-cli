use serde_json::{Value, json};

/// Reads the child issues in each `Scenario::Compaction` batch.
pub(super) const COMPACTION_READS_PER_BATCH: usize = 16;

/// The answer a resumed run receives once the provider is back.
pub(super) const RESUMED: &str = "Resumed: the run after the provider failure answered";

/// The wire shape of a scripted transport fault, from the client's view.
#[derive(Clone, Copy)]
pub(super) enum Fault {
    None,
    /// The request is read, then the socket goes away with no response at all:
    /// what an established-then-reset connection looks like (or a gateway that
    /// answers 520 with no body).
    Close,
    /// Headers and one SSE keep-alive chunk, then the socket goes away: the
    /// stream exists and dies without ever delivering content.
    Truncate,
}

#[derive(Clone, Copy)]
pub(super) enum Scenario {
    Feedback {
        batches: usize,
        width: usize,
    },
    Compaction {
        batches: usize,
    },
    /// The root's first request fails at the transport; every later request
    /// answers. Models the transient failure that must be retried.
    Transient {
        fault: Fault,
    },
    /// Every root request fails at the transport until `Provider::recover`, so
    /// the run spends its retries and ends on the provider failure a resume
    /// continues.
    Down {
        fault: Fault,
    },
    /// The root's first answer is a call the loop cannot use, so the run ends
    /// as an agent failure rather than a provider one.
    MalformedCall,
}
impl Scenario {
    /// The transport fault this request meets, if any.
    pub fn fault(self, _child: bool, stage: usize) -> Fault {
        match (self, stage) {
            (Self::Transient { fault }, 0) => fault,
            (Self::Down { fault }, _) => fault,
            _ => Fault::None,
        }
    }

    pub fn response(self, child: bool, stage: usize) -> String {
        if !child {
            match self {
                Self::Transient { .. } => {
                    return answer("Recovered: the retried request answered");
                }
                Self::Down { .. } => return answer(RESUMED),
                Self::MalformedCall => return malformed_call(),
                _ => {}
            }
            return match stage {
                // One synchronous `helpers` call: it launches the child, waits
                // for it, and returns its summary. The direct root has no
                // `coding_review` and no prepare tool: the owner prepares the
                // candidate from the root's own working checkout at the end of
                // the run, and the receipt it leaves behind is the review's
                // source of truth.
                0 => calls(vec![(
                    "task".into(),
                    "helpers",
                    json!({
                        "description":"write for child-context",
                        "prompt":"write for child-context; inspect actual files",
                        "subagent_type":"general",
                        "agent_id":"child-context",
                    }),
                )]),
                _ => answer("Actual context owner run finished"),
            };
        }
        if stage == 0 {
            return calls(vec![(
                "write".into(),
                "write",
                json!({"path":"new.txt","content":"owned context change\n"}),
            )]);
        }
        let (batch, batches, width) = match self {
            Self::Feedback { batches, width } => (stage - 1, batches, width),
            Self::Compaction { batches } => (stage - 1, batches, COMPACTION_READS_PER_BATCH),
            // The transient, down and malformed scenarios answer the root
            // only; a child never runs.
            Self::Transient { .. } | Self::Down { .. } | Self::MalformedCall => {
                return answer("Recovered: the retried request answered");
            }
        };
        if batch >= batches {
            return answer("Actual child context complete");
        }
        calls(
            (0..width)
                .map(|index| {
                    (
                        format!("read-{batch}-{index}"),
                        "read",
                        json!({"path":format!("padding-{index}.txt")}),
                    )
                })
                .collect(),
        )
    }
}
fn calls(calls: Vec<(String, &str, Value)>) -> String {
    let wire = calls
        .into_iter()
        .enumerate()
        .map(|(index, (id, name, args))| {
            json!({"index":index,"id":id,"type":"function","function":{
            "name":name,"arguments":args.to_string()}})
        })
        .collect::<Vec<_>>();
    sse(json!({"choices":[{"index":0,"delta":{"tool_calls":wire},"finish_reason":"tool_calls"}]}))
}
fn answer(text: &str) -> String {
    sse(json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":"stop"}]}))
}

/// A call with no id to answer and a tool the run does not advertise: the loop
/// cannot use it, so the run ends as an agent failure.
fn malformed_call() -> String {
    let wire = json!({"index":0,"id":"","type":"function","function":{
        "name":"no_such_tool","arguments":"{}"}});
    sse(json!({"choices":[{"index":0,"delta":{"tool_calls":[wire]},"finish_reason":"tool_calls"}]}))
}
fn sse(value: Value) -> String {
    format!(
        "data: {value}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}
