use super::ExtractionOutcome;
use crate::agent::learning::extraction_stream::{ExtractionStreamError, collect_extraction};
use crate::agent::learning::{
    ExtractionError, TurnRecord, build_extraction_prompt, filter_anti_self_reinforcement,
    ingest_proposals, parse_extraction_response, resolve_proposals,
};
use crate::connection::ConnectionRegistry;
use crate::contracts::RecallReceipt;
use saya_agent::{ChatProvider, ReasoningEffort, ResponseFormat};
use saya_store::KnowledgeItemStore;

/// Executes post-turn structured extraction using the given provider and persists resolved proposals.
pub(crate) async fn run_extraction(
    provider: &dyn ChatProvider,
    model: &str,
    record: &TurnRecord,
    registry: &ConnectionRegistry,
    store: &dyn KnowledgeItemStore,
    receipt: &RecallReceipt,
) -> ExtractionOutcome {
    if record.object_table.is_empty() {
        return ExtractionOutcome::ok(Vec::new(), None);
    }

    let request = build_extraction_prompt(record, model);
    // JSON mode lives here, not in the prompt builder: `build_extraction_prompt`
    // assembles the prompt; the
    // *policy* — "this is the extraction call, so the response must be a single
    // JSON object" — belongs to the caller that knows what the call is for.
    // On a reasoning model this stops the chain-of-thought we never read,
    // cutting the post-turn wait from seconds to ~1s. A provider that
    // cannot honour it degrades to today's behaviour (the prompt already asks
    // for JSON, `strip_markdown_fences` handles fences), never to an error.
    //
    // `Minimal` effort is the honest lever for the same goal — ask for less
    // thinking rather than suppressing it as a side effect of the JSON shape.
    // It is set on the `ChatRequest` so providers that translate it (Ollama's
    // `think: false`, Anthropic/Gemini's token budget) honour the ask. The
    // OpenAI-family wire, however, **drops** `Minimal`: the `"minimal"` spelling
    // is not universally honoured (a Fireworks-backed gateway rejects it with
    // HTTP 400, databook-studio/saya-cli#56), and the `ReasoningEffort` contract
    // is that a provider which cannot honour a variant drops it, never errors.
    // So on the OpenAI gateway this is effectively JSON-mode-only; the effort
    // lever still works on the providers that translate it. Whether the model
    // complied is only knowable from the reported reasoning tokens; saya reports
    // what it asked for, never that the effort was applied.
    let request = request
        .with_response_format(ResponseFormat::JsonObject)
        .with_reasoning_effort(ReasoningEffort::Minimal);
    // The reply is collected from the stream, not `complete`: a reply whose
    // first visible character is not JSON (a model ignoring JSON mode) stops
    // the provider at its first visible token instead of writing prose until
    // the output ceiling truncates it and the turn's timeout burns.
    let reply = match collect_extraction(provider, request).await {
        Ok(reply) => reply,
        Err(ExtractionStreamError::NotJson { usage }) => {
            return ExtractionOutcome::failed(ExtractionError::NotJson.into(), usage);
        }
        Err(ExtractionStreamError::Provider(error)) => {
            return ExtractionOutcome::failed(error.into(), None);
        }
    };
    let usage = reply.usage;
    let extracted = match parse_extraction_response(&reply.content, &record.object_table) {
        Ok(extracted) => extracted,
        Err(error) => return ExtractionOutcome::failed(error.into(), usage),
    };
    if extracted.is_empty() {
        return ExtractionOutcome::ok(Vec::new(), usage);
    }

    let resolved =
        resolve_proposals(extracted, &record.prompt, &record.object_table, registry).await;

    let filtered = filter_anti_self_reinforcement(resolved, &receipt.supplied);
    if filtered.is_empty() {
        return ExtractionOutcome::ok(Vec::new(), usage);
    }

    let fingerprint = crate::commands::unobserved_fingerprint();
    match ingest_proposals(store, filtered, fingerprint).await {
        Ok(dtos) => ExtractionOutcome::ok(dtos, usage),
        Err(error) => ExtractionOutcome::failed(error.into(), usage),
    }
}
