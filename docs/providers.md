# Providers

SAYA currently supports four provider interfaces:

| Provider | Runtime | Configuration |
| --- | --- | --- |
| Ollama | local HTTP service | `provider = "ollama"`, `model`, optional `base_url` |
| OpenAI-compatible | cloud or self-hosted HTTP chat-completions | `provider = "openai_compatible"`, `model`, `base_url`, `api_key` SecretRef |
| Anthropic | cloud HTTP API | `provider = "anthropic"`, `model`, `api_key` SecretRef, optional `base_url` |
| Gemini | cloud HTTP API (`x-goog-api-key`) | `provider = "gemini"`, `model`, `api_key` SecretRef, optional `base_url` |

The legacy `openai` provider name is also accepted for the OpenAI-compatible
path. API keys belong in an environment or file SecretRef, never as a literal
TOML value:

```toml
[ai]
provider = "openai_compatible"
model = "your-model"
base_url = "https://api.example.test/v1"
api_key = { env = "SAYA_API_KEY" }
```

```bash
SAYA_API_KEY=replace-me saya --non-interactive ask "summarize the schema"
```

Ollama is treated as local for this alpha. OpenAI-compatible, Anthropic, and
Gemini requests are treated as cloud; with data sharing disabled, schema metadata
may be available but SQL tools and rows are blocked. Provider failures return exit
code 5.

## Explicitly unavailable

- Fully offline agent use is not available: the CLI still requires a configured
  provider endpoint for agent responses, even when the database is local.
- API-key storage, provider-specific tool protocols, and model downloads are
  outside this CLI; configure those through the provider runtime.

Provider output is streamed as stable text, JSON, or NDJSON events. A partially
consumed HTTP response is not retried because replaying it cannot be proved
action-free.
