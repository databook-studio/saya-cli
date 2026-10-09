use futures_util::StreamExt;
use saya_agent::{
    CancellationToken, ChatMessage, ChatProvider, ChatRequest, LocalStateEffect, OllamaProvider,
    OpenAiCompatibleProvider, ProviderError, ProviderEvent, ProviderSettings, TokenUsage, ToolCall,
    ToolDefinition, ToolEffect,
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
include!("providers/reply.rs");
include!("providers/server.rs");
include!("providers/byte_server.rs");
include!("providers/keep_open_server.rs");
include!("providers/silent_server.rs");
include!("providers/read_request.rs");
include!("providers/request.rs");
include!("providers/openai.rs");
include!("providers/redirect_capture.rs");
include!("providers/redirect_capture_server.rs");
include!("providers/await_capture.rs");
include!("providers/drain.rs");
include!("providers/drain_reasoning.rs");
include!("providers/drain_results.rs");
include!("providers/openai_sse_handles_fragmented_text_and_done.rs");
include!("providers/openai_sse_ping_frames_stop_at_the_shared_frame_limit.rs");
include!("providers/anthropic_and_ollama_empty_records_share_the_frame_limit.rs");
include!("providers/openai_sse_preserves_utf8_and_queued_delta_before_done.rs");
include!("providers/terminal_sentinels_allow_only_trailing_ascii_whitespace.rs");
include!("providers/terminal_markers_finish_without_waiting_for_socket_close.rs");
include!("providers/openai_sse_assembles_fragmented_tool_calls.rs");
include!(
    "providers/anthropic_stream_refuses_more_than_256_sparse_tool_blocks_before_emitting_calls.rs"
);
include!("providers/anthropic_stream_keeps_sparse_indexes_below_the_call_cap.rs");
include!("providers/anthropic_stream_keeps_a_valid_id_with_non_object_arguments.rs");
include!("providers/anthropic_stream_refuses_tool_identifiers_over_the_byte_limit.rs");
include!("providers/ollama_ndjson_handles_fragmentation_and_requires_done.rs");
include!("providers/ollama_accepts_terminal_record_without_trailing_newline.rs");
include!("providers/retries_before_first_event_but_not_after_partial_stream.rs");
include!("providers/cancellation_and_errors_are_sanitized.rs");
include!("providers/every_streaming_provider_applies_the_establishment_deadline.rs");
include!("providers/anthropic_stream_reports_stall_on_idle_timeout.rs");
include!("providers/openai_stream_reports_stall_on_idle_timeout.rs");
include!("providers/ollama_stream_reports_stall_on_idle_timeout.rs");
include!("providers/anthropic_stream_surfaces_cumulative_token_usage.rs");
include!(
    "providers/anthropic_complete_normalizes_cache_counts_and_preserves_unknown_zero_and_overflow.rs"
);
include!("providers/openai_stream_surfaces_usage_and_requests_it.rs");
include!("providers/openai_usage_only_zero_and_absent_usage_survive_complete_and_stream.rs");
include!("providers/openai_empty_choices_without_current_usage_remain_invalid_after_a_receipt.rs");
include!("providers/gemini_complete_normalizes_thought_tokens_without_double_counting_metadata.rs");
include!("providers/ollama_stream_surfaces_eval_counts.rs");
include!("providers/ollama_complete_distinguishes_reported_zero_from_absent_usage.rs");
include!("providers/ollama_frame_limit_does_not_parse_usage_from_the_rejected_record.rs");
include!("providers/ollama_frame_limit_precedes_json_conversion_of_the_rejected_record.rs");
include!("providers/length_truncation_is_diagnosable_not_generic.rs");
include!("providers/openai_length_truncation_is_a_typed_error_carrying_partial_text.rs");
include!("providers/ollama_length_done_reason_is_a_typed_truncation_error.rs");
include!("providers/ollama_complete_and_stream_send_configured_num_predict.rs");
include!("providers/openai_body_carries_max_completion_tokens.rs");
include!("providers/unbounded_frames_fail_at_the_stream_byte_cap.rs");
include!("providers/unreachable_provider_error_names_the_endpoint.rs");
include!("providers/openai_stream_captures_reasoning_content.rs");
include!("providers/openai_stream_without_reasoning_leaves_it_none.rs");
include!("providers/anthropic_stream_captures_thinking_delta.rs");
include!("providers/anthropic_stream_without_thinking_leaves_reasoning_none.rs");
include!("providers/ollama_stream_captures_thinking.rs");
include!("providers/ollama_stream_without_thinking_leaves_reasoning_none.rs");
include!("providers/ollama_does_not_replay_the_post_to_a_cross_host_redirect.rs");
include!(
    "providers/default_client_replays_body_and_bespoke_key_headers_but_strips_authorization.rs"
);
include!("providers/openai_does_not_replay_the_post_to_a_cross_host_redirect.rs");
include!("providers/anthropic_does_not_replay_the_post_to_a_cross_host_redirect.rs");
include!("providers/gemini_does_not_replay_the_post_to_a_cross_host_redirect.rs");
include!("providers/ollama_truncated_stream_reports_usage_before_the_typed_error.rs");
include!("providers/ollama_truncated_stream_without_usage_emits_no_usage_event.rs");
include!("providers/openai_truncated_stream_reports_same_frame_usage_before_the_typed_error.rs");
include!("providers/openai_truncated_stream_without_usage_emits_no_usage_event.rs");
include!("providers/openai_truncated_stream_charges_a_trailing_usage_only_frame.rs");
include!("providers/openai_truncated_stream_without_done_still_ends_in_the_typed_error.rs");
include!("providers/openai_truncated_stream_ignores_a_garbage_trailer.rs");
include!("providers/openai_queued_usage_survives_a_malformed_frame_in_the_same_read.rs");
