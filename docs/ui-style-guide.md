# saya TUI style guide

saya follows the **Claude Code / Anthropic design language** — a warm, low-chroma
palette, a single accent used sparingly, secondary content that recedes, and status
coded by meaning — but with its **own signature colour** rather than Anthropic's
orange. saya's mark is a refined **iris violet**. Design-language sources: Claude
Code theme-token docs (`code.claude.com/docs/en/terminal-config`) and Anthropic's
brand-guidelines (`github.com/anthropics/skills`).

## Palette

| Name | Hex | Role |
| --- | --- | --- |
| **Iris (accent)** | `#9d8bf5` | saya's signature — brand mark, **assistant**, input border, focus, scrollbar, selection |
| Blue | `#6a9bcc` | **User** turns (a cool secondary, so the accent stays saya's) |
| Text | `#faf9f5` | Primary foreground (assistant body) |
| Warm gray (secondary) | `#a8a29a` | Tool/step lines, the SQL echo, hints, timestamps, provider/model |
| Green (success) | `#7fae6b` | Passing checks, `read-only` approval, `privacy:on` |
| Amber (warning) | `#e0a458` | `ask` approval, the approval-panel border, caution |
| Red (error) | `#e5695f` | Failures, `never` approval |
| Code blue | `#7fb5d6` | Inline `` `code` `` in answers |
| Status/badge bg | `#1e1c24` | Status-bar strip (a faint iris-tinted dark) |
| Base bg reference | `#141413` | Terminal background it's tuned against |

## Principles

- **One accent, used sparingly.** Iris marks the brand, the assistant, and the active
  input border — not large fills. A little accent goes a long way; overusing it reads
  as an alert.
- **The accent belongs to saya, not you.** The signature colour is the *assistant*;
  the user is a cooler blue so the two turns read as distinct without competing for the
  brand colour.
- **Secondary content recedes.** Tool/step lines, the SQL echo, and hints sit in warm
  gray so the answer is what stands out.
- **Status is coded by meaning, not decoration.** Green = safe/success, amber = needs
  attention, red = error. Approval mode follows this: read-only green, ask amber, never red.
- **Re-tintable.** Everything keys off a small set of named constants at the top of
  `ui.rs`; changing saya's signature is a one-line edit to the `ACCENT` constant.

## Mapping to the code (`crates/saya-cli/src/interactive/tui/ui.rs`)

| Element | Colour |
| --- | --- |
| `ACCENT` const → input border, splash title, scrollbar thumb, popup/picker/help borders, profile segment, bullets | Iris `#9d8bf5` |
| Assistant rail | Iris `#9d8bf5` |
| User rail + user text | Blue `#6a9bcc` |
| Assistant body text | `#faf9f5` (terminal default light) |
| Tool lines / SQL echo / system lines / hints / provider·model | Warm gray `#a8a29a` |
| Error lines | Red `#e5695f` |
| Status-bar background | `#1e1c24` |
| Status: approval read-only / ask / never | Green `#7fae6b` / Amber `#e0a458` / Red `#e5695f` |
| Status: privacy on / off | Green `#7fae6b` / warm gray `#a8a29a` |
| Approval panel border | Amber `#e0a458` |
| Inline `code` in answers | Code blue `#7fb5d6` |
| Selection-mode badge | Iris bg, black text |

Tuned for dark terminals (the common case); the hues stay legible on light backgrounds too.
