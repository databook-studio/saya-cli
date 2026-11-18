//! Part-B extensions to the core record: the recall block a request carried,
//! the post-turn learning counts, and the saved-investigation replay.
//!
//! These live apart from `record.rs` so each file holds one concern: the core
//! record stays the part-A contract, and this module is everything the part-B
//! scenarios added to it.

use super::record::{Golden, InvestigationRecord, LearningRecord, RecallRecord};

impl Golden {
    /// Records how many claims the extractor offered and how many the user saw
    /// stored. The extraction-request count comes from the bodies, so a
    /// scenario cannot claim extraction happened when it did not.
    pub(crate) fn with_learning_counts(mut self, proposed: usize, stored: usize) -> Self {
        let learning = self.learning.get_or_insert(LearningRecord {
            extraction_requests: 0,
            proposed: 0,
            stored: 0,
        });
        learning.proposed = proposed;
        learning.stored = stored;
        self
    }

    /// Attaches a saved-investigation replay. Its `requests` stay empty: this
    /// row never touches a provider.
    pub(crate) fn with_investigation(mut self, investigation: InvestigationRecord) -> Self {
        self.investigation = Some(investigation);
        self
    }
}

/// The recall block in a request, if the user turn carries one. The block is
/// rendered into the first user message and replayed in later requests, so
/// each request is inspected rather than only the first.
pub(crate) fn recall_block(value: &serde_json::Value) -> Option<RecallRecord> {
    let messages = value["messages"].as_array()?;
    messages.iter().find_map(|message| {
        if message["role"] != "user" {
            return None;
        }
        let content = message["content"].as_str()?;
        recall_body_bytes(content).map(|body_bytes| RecallRecord {
            present: true,
            body_bytes,
        })
    })
}

/// Bytes between the `database-contracts` source line and the closing
/// sentinel. The sentinels are `saya-agent`'s; the body is already normalised
/// because the whole request was.
fn recall_body_bytes(content: &str) -> Option<usize> {
    const OPEN: &str = "<<<CONTEXT_BLOCK_BEGIN>>>\n";
    const SOURCE: &str = "source: database-contracts\n";
    const CLOSE: &str = "<<<CONTEXT_BLOCK_END>>>";
    let after_open = content.find(OPEN)? + OPEN.len();
    let after_source = content[after_open..].strip_prefix(SOURCE)?;
    let body = &after_source[..after_source.find(CLOSE)?];
    Some(body.len())
}

/// Whether this request is the post-turn extractor's: the extractor is the one
/// caller that asks for a JSON object and names its role in the prompt.
pub(crate) fn is_extraction_request(value: &serde_json::Value) -> bool {
    if value["response_format"]["type"].as_str() != Some("json_object") {
        return false;
    }
    value["messages"].as_array().is_some_and(|messages| {
        messages.iter().any(|message| {
            message["content"]
                .as_str()
                .is_some_and(|content| content.contains("schema knowledge extractor"))
        })
    })
}
