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

Each agent tool carries typed capability flags (`ToolEffect { database_data,
external_side_effect, requires_approval }`). Data sharing being disabled for a
cloud provider filters out every tool with `database_data` (the bounded-SQL
tools), while `requires_approval` tools (the bounded-SQL family) are
gated by the approval policy before they run.

## Explicitly unavailable

- Fully offline agent use is not available: the CLI still requires a configured
  provider endpoint for agent responses, even when the database is local.
- API-key storage, provider-specific tool protocols, and model downloads are
  outside this CLI; configure those through the provider runtime.

Provider output is streamed as stable text, JSON, or NDJSON events. A partially
consumed HTTP response is not retried because replaying it cannot be proved
action-free.

A reply cut at the output cap is typed, not silent: every provider maps its own
limit signal (`max_tokens`, `length`, `MAX_TOKENS`) to
`ProviderError::OutputTruncated` carrying the partial output. The turn ends
there rather than retrying, and any usage the provider reported ahead of the
cut is still charged. Each truncation is counted and surfaces in the
`--verbose` metrics section.

A stream that ends before its terminator is split by what it delivered. A
stream that delivered **nothing** is a `ProviderError::StreamEnded`: the client
cannot tell what the provider did, so the attempt is retried on the configured
backoff, exactly as before. A stream that had already delivered content bytes —
text, reasoning, or admitted tool-call arguments — is a **stream cut**
(`ProviderError::StreamCut`): a gateway response time limit cuts the identical
request at the same point, so the byte-identical request is never re-sent.
Within the continuation allowance the run instead spends one bounded
continuation whose closed user-role note names the window the provider allowed;
the incomplete output is discarded, so reasoning never re-enters request
history and a partial tool call never executes. The note does not order one
tool call or smaller steps. A cut reports no usage (those rows show
`usage: null`). `estimated_tokens` is `ceil(visible_bytes / 3.5)` from text
and admitted tool-call arguments only — reasoning is discarded, so it is not
a measured output window even though it proves the stream flowed. Elapsed
seconds come from the stream's own clock. If the allowance is spent the cut
settles the run as `ProviderStreamEnded`, the same class and operator
guidance a terminator-less stream with no content gets.

A time-cut does not change the output ceiling. The configured
`ai.max_output_tokens` is what later requests ask for. A ~300 s gateway cut
is a clock, including one that leaked a scrap of visible text; that scrap is
not an output window, and the loop does not fit a later request to 85% of it
or to a 1,024 floor. The first request is still fitted to 16,384 when the
operator configured more than that. Output-token truncation
(`ProviderError::OutputTruncated`) is a different failure and does not take
this path.

The cut is never silent. Each admitted continuation emits a
`stream_cut_recovered` event (and a matching `--verbose`/NDJSON line, and the
child's own progress channel) carrying three closed fields: `elapsed_secs`,
`estimated_tokens`, and `ceiling_after`. `ceiling_after` is the configured
ceiling the run continues under. `estimated_tokens` tells the operator what
visible text arrived; it does not size the next request.
