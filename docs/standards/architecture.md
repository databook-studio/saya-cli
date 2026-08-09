# Architecture standard

How the workspace is layered and where new code belongs. The goal is that any change
has an **obvious home**, and dependencies only ever point one way.

## Crate graph (dependencies point down)

```
                      saya-cli            composition + all presentation (TUI, render, CLI)
                    /   |    |   \
        saya-agent  saya-connectors  saya-store        capabilities
              \          |          /
               \    saya-config    /                   configuration
                \       |         /
                 \      |        /
                    saya-types                          contracts (leaf, no saya deps)
```

Actual edges (keep them this way):

| Crate | May depend on | Responsibility |
| --- | --- | --- |
| `saya-types` | *(nothing internal)* | Shared contracts: profiles, `QueryResult`, `SchemaTree`, `ConnectionError`, dialects. Pure data + errors. |
| `saya-config` | `types` | Config resolution: layers, env files, secret references, diagnostics. |
| `saya-store` | `types` | Persistence: sessions, history, audit, schema cache, redaction. |
| `saya-agent` | `types` | Provider protocol + agent loop (Anthropic, OpenAI, Gemini, Ollama). |
| `saya-connectors` | `config`, `types` | Database connectors **and** the read-only SQL safety layer. |
| `saya-cli` | all of the above | The binary: TUI, `render`, slash/CLI parsing, wiring. **All presentation.** |

## Rules

- **Dependencies point down, never up or sideways at the same layer.** `saya-config`
  must not learn about `saya-connectors`; `saya-agent` and `saya-store` don't know
  about each other. If two mid-layer crates need to share, the shared thing is a
  *contract* and belongs in `saya-types`.
- **Contracts live in the crate that owns them, not in `saya-cli`.** A new connector
  capability, config field, or provider type is defined in its crate and re-exported;
  `saya-cli` consumes it.
- **Presentation lives only in `saya-cli`.** Formatting, colour, TUI widgets, and
  human/JSON rendering never leak into a library crate. Library crates return data
  and typed errors; `saya-cli` decides how it looks. (See `render::render_event`.)
- **New connectors/providers stay behind honest capability boundaries** until
  contract, integration, and security tests exist ([security.md](security.md),
  [testing.md](testing.md)). Don't advertise a capability the tests don't cover.

## Modules and file size

- **New production `.rs` files ≤ 150 lines (soft target), 250 lines hard cap.** When a
  file grows past the soft target, split by *concern*, not by line count — mirror the
  existing pattern of a thin `mod.rs` plus focused submodules (e.g.
  `connection/{mod,registry,build}.rs`, `providers/{anthropic,openai,gemini}.rs`).
- **Keep `mod.rs` small** — declarations, the public surface, and glue. Logic goes in
  named submodules.
- **Unit tests may live inline** (`mod tests` or a sibling `*_tests.rs`); they don't
  count against a source file's budget but still should be split when large. Cross-crate
  behavior goes in `tests/` ([testing.md](testing.md)).

## Adding a crate

Rare. Only when a responsibility is genuinely new *and* would otherwise force an
upward dependency. Add it to the workspace `members`, give it the workspace `edition`/
`rust-version`/`license`, place it at the correct layer, and document its row in the
table above in the same PR.
