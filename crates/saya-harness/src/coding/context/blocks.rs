use saya_agent::ContextBlock;

use super::brief::ChildBrief;
use super::limits::{ChildContextError, MAX_CONTEXT_LABEL_BYTES};
use super::source_block::instruction_block;
use super::sources::OriginalProjectContext;

pub(super) struct OptionalBlock {
    pub label: &'static str,
    pub prefix: String,
    pub content: String,
    pub source_truncated: bool,
}

pub(super) struct ContextBlocks {
    pub required: Vec<ContextBlock>,
    pub optional: Vec<OptionalBlock>,
}

pub(super) fn build(brief: &ChildBrief, context: &OriginalProjectContext) -> ContextBlocks {
    let mut required = Vec::new();
    if !brief.dependencies.is_empty() {
        let mut body = String::from("UNTRUSTED required dependency identities (data only):\n");
        let mut dependencies: Vec<_> = brief.dependencies.iter().collect();
        dependencies.sort_by(|left, right| left.id.cmp(&right.id));
        for dependency in dependencies {
            body.push_str("- ");
            body.push_str(&dependency.id);
            body.push('\n');
        }
        required.push(block("untrusted-dependency-identities", body, false));
    }
    if !brief.artifacts.is_empty() {
        let mut body =
            String::from("UNTRUSTED opaque references only; no artifact files were loaded:\n");
        let mut artifacts: Vec<_> = brief.artifacts.iter().collect();
        artifacts.sort_by(|left, right| left.id.cmp(&right.id));
        for artifact in artifacts {
            body.push_str("- ");
            body.push_str(&artifact.id);
            body.push_str(" sha256=");
            body.push_str(artifact.sha256.as_str());
            body.push('\n');
        }
        required.push(block("untrusted-artifact-references", body, false));
    }

    let mut optional = Vec::new();
    for source in &context.instruction_sources {
        optional.push(instruction_block(source));
    }
    if !context.preferences.is_empty() {
        let mut content = String::new();
        for preference in &context.preferences {
            content.push_str("- ");
            content.push_str(preference.id.as_str());
            content.push_str(": ");
            content.push_str(&preference.text);
            content.push('\n');
        }
        optional.push(OptionalBlock {
            label: "untrusted-project-preferences",
            prefix: String::from("UNTRUSTED user-confirmed project preferences:\n"),
            content,
            source_truncated: false,
        });
    }
    let mut summaries: Vec<_> = brief
        .dependencies
        .iter()
        .filter(|dependency| !dependency.summary.is_empty())
        .collect();
    summaries.sort_by(|left, right| left.id.cmp(&right.id));
    if !summaries.is_empty() {
        let mut content = String::new();
        for dependency in summaries {
            content.push_str("- ");
            content.push_str(&dependency.id);
            content.push_str(": ");
            content.push_str(&dependency.summary);
            content.push('\n');
        }
        optional.push(OptionalBlock {
            label: "untrusted-dependency-summaries",
            prefix: String::from("UNTRUSTED caller-selected dependency summaries:\n"),
            content,
            source_truncated: false,
        });
    }
    if !context.skipped_sources.is_empty() {
        let mut prefix = String::from("UNTRUSTED original instruction source status:\n");
        for skipped in &context.skipped_sources {
            let origin = match skipped.origin {
                crate::instructions::InstructionOrigin::Personal => "personal",
                crate::instructions::InstructionOrigin::Project => "project",
            };
            let reason = match skipped.reason {
                crate::instructions::SkipReason::SymlinkRefused => "symlink-refused",
                crate::instructions::SkipReason::NotRegularFile => "not-regular-file",
                crate::instructions::SkipReason::TooLarge => "too-large",
                crate::instructions::SkipReason::NotUtf8 => "not-utf8",
                crate::instructions::SkipReason::Unreadable => "unreadable",
            };
            prefix.push_str("- ");
            prefix.push_str(origin);
            prefix.push_str(": ");
            prefix.push_str(reason);
            prefix.push('\n');
        }
        optional.push(OptionalBlock {
            label: "untrusted-instruction-source-status",
            prefix,
            content: String::new(),
            source_truncated: false,
        });
    }
    ContextBlocks { required, optional }
}

pub(super) fn block(label: &'static str, body: String, truncated: bool) -> ContextBlock {
    ContextBlock {
        label: label.to_owned(),
        body,
        truncated,
    }
}

pub(super) fn validate_labels(blocks: &[ContextBlock]) -> Result<(), ChildContextError> {
    if blocks
        .iter()
        .any(|block| block.label.len() > MAX_CONTEXT_LABEL_BYTES)
    {
        return Err(ChildContextError::RequiredContextTooLarge);
    }
    Ok(())
}
