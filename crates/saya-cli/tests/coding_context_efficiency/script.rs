use serde_json::{Value, json};

/// Reads the child issues in each `Scenario::Compaction` batch.
pub(super) const COMPACTION_READS_PER_BATCH: usize = 16;

#[derive(Clone, Copy)]
pub(super) enum Scenario {
    Feedback { batches: usize, width: usize },
    Compaction { batches: usize },
}
impl Scenario {
    pub fn response(self, child: bool, stage: usize) -> String {
        if !child {
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
fn sse(value: Value) -> String {
    format!(
        "data: {value}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}
