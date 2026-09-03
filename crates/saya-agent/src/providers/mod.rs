mod anthropic;
mod anthropic_request;
mod anthropic_stream;
mod framing;
mod gemini;
mod gemini_request;
mod gemini_response;
mod http;
mod ollama;
mod ollama_chunks;
mod ollama_stream;
mod ollama_wire;
mod openai;
mod openai_chunks;
mod openai_stream;
mod settings;
mod tool_assembly;
mod wire;

pub use anthropic::AnthropicProvider;
pub use gemini::GeminiProvider;
pub use ollama::OllamaProvider;
pub use openai::OpenAiCompatibleProvider;
pub use settings::ProviderSettings;

#[cfg(test)]
mod context_block_tests {
    use super::{
        anthropic_request, gemini_request, ollama_wire,
        wire::{self, WireMessage},
    };
    use crate::{
        ChatMessage, ChatRequest, ContextBlock, LocalStateEffect, ToolDefinition, ToolEffect,
        history::build_messages,
        history_context::{CONTEXT_CLOSE, CONTEXT_OPEN},
    };

    /// A distinct body that does not appear in the SAYA system prompt or the
    /// preamble, so "absent from the system field" is a meaningful assertion.
    const BODY: &str = "CLAIM_SENTINEL_BODY_9f3a";

    fn block() -> ContextBlock {
        ContextBlock {
            label: "database-contracts".into(),
            body: BODY.into(),
            truncated: false,
        }
    }

    fn definitions() -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            name: "schema_discovery".into(),
            description: "schema".into(),
            read_only: true,
            parameters: serde_json::json!({"type": "object"}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            },
        }]
    }

    /// Messages shaped by `build_messages` with one context block.
    fn messages() -> Vec<ChatMessage> {
        build_messages(None, &[block()], "real prompt", &[], 32 * 1024).unwrap()
    }

    fn request(messages: Vec<ChatMessage>) -> ChatRequest {
        ChatRequest {
            model: "m".into(),
            messages,
            tools: definitions(),
            ..Default::default()
        }
    }

    /// Every system-role message in the wire must be free of the block body, and the
    /// block body must appear in a user message, inside the wrapper. Generic over the
    /// OpenAI (`WireMessage`) and Ollama (`OllamaMessage`) shapes, which differ in
    /// type but both expose `role` and `content`.
    fn assert_no_block_in_system_messages<T>(
        wire_messages: &[T],
        role: fn(&T) -> &str,
        content: fn(&T) -> &str,
    ) {
        for message in wire_messages {
            if role(message) == "system" {
                assert!(
                    !content(message).contains(BODY),
                    "context block body leaked into a system-role message"
                );
                assert!(
                    !content(message).contains(CONTEXT_OPEN),
                    "context wrapper leaked into a system-role message"
                );
            }
        }
        let user_has_block = wire_messages
            .iter()
            .filter(|m| role(m) == "user")
            .any(|m| content(m).contains(BODY) && content(m).contains(CONTEXT_OPEN));
        assert!(
            user_has_block,
            "context block body must appear in a user message inside the wrapper"
        );
    }

    fn wire_role(m: &WireMessage) -> &str {
        &m.role
    }
    fn wire_content(m: &WireMessage) -> &str {
        &m.content
    }
    fn ollama_role(m: &super::ollama_wire::OllamaMessage) -> &str {
        &m.role
    }
    fn ollama_content(m: &super::ollama_wire::OllamaMessage) -> &str {
        &m.content
    }

    #[test]
    fn anthropic_request_keeps_context_block_out_of_system_field() {
        let body = anthropic_request::build_body(request(messages()), 1024, None);
        // Anthropic hoists system-role messages into the top-level `system` string.
        assert!(
            !body["system"].as_str().unwrap_or("").contains(BODY),
            "context block body leaked into the Anthropic system field"
        );
        assert!(
            !body["system"].as_str().unwrap_or("").contains(CONTEXT_OPEN),
            "context wrapper leaked into the Anthropic system field"
        );
        // The block body is in a user message, inside the wrapper.
        let user_text = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "user")
            .map(|m| m["content"].as_str().unwrap_or(""))
            .collect::<String>();
        assert!(user_text.contains(BODY));
        assert!(user_text.contains(CONTEXT_OPEN));
        assert!(user_text.contains(CONTEXT_CLOSE));
        // The tool list is unaffected by context blocks.
        assert_eq!(body["tools"][0]["name"], "schema_discovery");
    }

    #[test]
    fn gemini_request_keeps_context_block_out_of_system_instruction() {
        let body = gemini_request::build_body(request(messages()), 4096, None);
        let system_text = body["systemInstruction"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("");
        assert!(
            !system_text.contains(BODY),
            "context block body leaked into the Gemini systemInstruction"
        );
        assert!(
            !system_text.contains(CONTEXT_OPEN),
            "context wrapper leaked into the Gemini systemInstruction"
        );
        let user_text = body["contents"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "user")
            .flat_map(|m| m["parts"].as_array().unwrap().iter())
            .map(|p| p["text"].as_str().unwrap_or(""))
            .collect::<String>();
        assert!(user_text.contains(BODY));
        assert!(user_text.contains(CONTEXT_OPEN));
        assert!(user_text.contains(CONTEXT_CLOSE));
        assert_eq!(
            body["tools"][0]["functionDeclarations"][0]["name"],
            "schema_discovery"
        );
    }

    #[test]
    fn openai_wire_keeps_context_block_out_of_system_messages() {
        let wire_messages = wire::messages(messages());
        assert_no_block_in_system_messages(&wire_messages, wire_role, wire_content);
        // Tools are shaped independently of messages; the block does not alter them.
        let tools = wire::tools(definitions());
        assert_eq!(tools[0].function.name, "schema_discovery");
    }

    #[test]
    fn ollama_wire_keeps_context_block_out_of_system_messages() {
        let ollama = ollama_wire::request(request(messages()));
        assert_no_block_in_system_messages(&ollama.messages, ollama_role, ollama_content);
        assert_eq!(ollama.tools[0].function.name, "schema_discovery");
    }
}
