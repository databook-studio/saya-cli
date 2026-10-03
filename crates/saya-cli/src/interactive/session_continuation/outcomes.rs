use saya_agent::{AgentOutput, ToolMetadata};

const MAX_TOOL_OUTCOMES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolOutcomeKind {
    Completed,
    Failed,
    Denied,
    RecordedUnverified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolOutcome {
    pub(crate) kind: ToolOutcomeKind,
    pub(crate) result_count: Option<u64>,
}

/// Outcomes mapped only while the actual live `AgentOutput` is being recorded.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PriorToolOutcomes {
    pub(crate) values: Vec<ToolOutcome>,
    pub(crate) truncated: bool,
}

impl PriorToolOutcomes {
    pub(crate) fn from_live_output(output: &AgentOutput) -> Self {
        let metadata = &output.tool_metadata;
        let truncated = metadata.len() > MAX_TOOL_OUTCOMES;
        let values = metadata
            .iter()
            .take(MAX_TOOL_OUTCOMES)
            .map(outcome_from_metadata)
            .collect();
        Self { values, truncated }
    }
}

fn outcome_from_metadata(tool: &ToolMetadata) -> ToolOutcome {
    ToolOutcome {
        kind: match tool.status.as_str() {
            "completed" => ToolOutcomeKind::Completed,
            "failed" => ToolOutcomeKind::Failed,
            "denied" => ToolOutcomeKind::Denied,
            _ => ToolOutcomeKind::RecordedUnverified,
        },
        result_count: tool.result_shape.as_ref().map(|shape| shape.row_count),
    }
}
