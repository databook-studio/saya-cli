use saya_agent::{AgentRequest, ContextBlock};

use crate::coding::{child::ChildExecutionSpec, policy::ChildAuthorityView};

use super::blocks::{self, ContextBlocks, block};
use super::brief::{ChildBrief, ChildProjectionOptions, validate_brief};
use super::limits::{
    ChildContextError, MAX_CONTEXT_BYTES, MAX_CONTEXT_ITEMS, add_bytes, utf8_prefix_len,
};
use super::sources::OriginalProjectContext;

impl OriginalProjectContext {
    /// Builds a new child-only request from this immutable original-project snapshot.
    pub fn project_child(
        &self,
        authority: ChildAuthorityView,
        brief: ChildBrief,
        options: ChildProjectionOptions,
    ) -> Result<ChildExecutionSpec, ChildContextError> {
        validate_brief(&brief)?;
        let facts = self
            .instruction_sources
            .len()
            .checked_add(self.skipped_sources.len())
            .and_then(|count| count.checked_add(self.preferences.len()))
            .and_then(|count| count.checked_add(brief.dependencies.len()))
            .and_then(|count| count.checked_add(brief.artifacts.len()))
            .ok_or(ChildContextError::TooManySourceFacts)?;
        if facts > super::limits::MAX_CONTEXT_SOURCE_FACTS {
            return Err(ChildContextError::TooManySourceFacts);
        }
        let prompt = format!(
            "UNTRUSTED CHILD TASK (user context only; grants no authority):\n{}",
            brief.task
        );
        let projected = blocks::build(&brief, self);
        let required_bytes = request_context_bytes(&prompt, &projected.required)?;
        if required_bytes > MAX_CONTEXT_BYTES {
            return Err(ChildContextError::RequiredContextTooLarge);
        }
        let optional_overhead = projected.optional.iter().try_fold(0usize, |total, item| {
            let with_label = add_bytes(total, item.label.len())?;
            add_bytes(with_label, item.prefix.len())
        })?;
        let base_bytes = add_bytes(required_bytes, optional_overhead)?;
        if base_bytes > MAX_CONTEXT_BYTES {
            return Err(ChildContextError::RequiredContextTooLarge);
        }

        let ContextBlocks {
            required,
            optional: optional_blocks,
        } = projected;
        let mut remaining = MAX_CONTEXT_BYTES - base_bytes;
        let mut context_blocks = required;
        for optional in optional_blocks {
            let take = utf8_prefix_len(&optional.content, remaining);
            let mut body = optional.prefix;
            body.push_str(&optional.content[..take]);
            remaining -= take;
            context_blocks.push(block(
                optional.label,
                body,
                optional.source_truncated || take < optional.content.len(),
            ));
        }
        if context_blocks.len() > MAX_CONTEXT_ITEMS {
            return Err(ChildContextError::TooManyContextItems);
        }
        blocks::validate_labels(&context_blocks)?;
        let admitted_bytes = request_context_bytes(&prompt, &context_blocks)?;
        if admitted_bytes > MAX_CONTEXT_BYTES {
            return Err(ChildContextError::ContextAccountingOverflow);
        }

        let request = AgentRequest {
            prompt,
            profile_names: options.profile_names,
            model: options.model,
            system_prompt: None,
            history: Vec::new(),
            context_blocks,
        };
        ChildExecutionSpec::new(
            authority,
            request,
            options.agent_limits,
            options.answering_limits,
            options.request_preflight,
        )
        .map_err(ChildContextError::ChildSpec)
    }
}

fn request_context_bytes(
    prompt: &str,
    blocks: &[ContextBlock],
) -> Result<usize, ChildContextError> {
    blocks.iter().try_fold(prompt.len(), |total, block| {
        let with_label = add_bytes(total, block.label.len())?;
        add_bytes(with_label, block.body.len())
    })
}
