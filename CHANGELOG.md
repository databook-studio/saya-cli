# Changelog

All notable changes to SAYA CLI are recorded here.

## Unreleased

- Added multi-database agent navigation: connect additional read-only databases
  alongside the primary with `--include-profile` (and interactive `/include`),
  and the AI agent inspects and queries any connected database by passing an
  optional `connection` argument to its tools. The agent is told the name and
  SQL dialect of every connected database; a failed secondary connection is
  skipped while the primary run continues.
- Added a native Anthropic (Claude) provider (`provider = "anthropic"`):
  streaming `content_block` parsing, `input_schema` tool declarations, top-level
  `system`, and `x-api-key`/`anthropic-version` headers.
- Added a Google Gemini provider (`provider = "gemini"`): buffered
  `generateContent` with `functionDeclarations`, `systemInstruction`, and the
  `x-goog-api-key` header. All five documented providers (Ollama,
  OpenAI-compatible, OpenAI, Anthropic, Gemini) are now implemented, so
  configuration and runtime agree.
- Added an interactive status header above the `saya> ` prompt showing the
  active profile, included databases, provider/model, approval mode, and privacy
  state.
- Cloud row-data sharing for Anthropic and Gemini is gated on
  `--allow-data-sharing`, consistent with the other cloud providers.
- Added `saya config init` for credential-free project templates.
- Added local archive packaging with checksum and extracted-binary smoke tests.
- Added a manually triggered release-candidate workflow for native CI builds.
- Documented the supported provider, installation, configuration, connection, and
  release boundaries.

## 0.1.0

- Initial private-alpha CLI surface for PostgreSQL, MySQL, DuckDB, Snowflake,
  Ollama, and OpenAI-compatible providers.
