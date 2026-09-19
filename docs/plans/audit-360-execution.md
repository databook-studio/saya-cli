# Audit 360 execution plan

## Context

Implement the 51 retained audit findings documented in the private audit and its
implementation plan. The intended outcome is a safer, bounded, honest terminal
agent without changing the read-only database guarantee or silently widening
workspace authority.

## Scope

Work through the 33 ordered slices from `audit-360-implementation.md`: localized
CLI/chart/config/release corrections first, followed by configuration and store
validation, provider and connector resource bounds, workspace harness bounds, and
the knowledge/store contract migrations. Each slice remains independently
reviewable and commits only its own production code, regression tests, and required
user-facing documentation.

Out of scope: new database write bypasses, network probing for `config doctor`,
unapproved changes to workspace approval policy, release publication, use of real
credentials, and live database/provider tests outside their existing opt-in gates.

## Crates and ownership

- `saya-cli`: presentation, session outcomes, charts, exports, consent adapters,
  tool schemas, and turn-file handling.
- `saya-config`: effective diagnostics, secret and endpoint validation.
- `saya-store`: checked persistence conversions, atomic bounded persistence, and
  startup/erasure classification.
- `saya-agent`: provider deadline, cancellation, and decoding limits.
- `saya-connectors`: bounded connector decoding and truthful schema discovery;
  read-only SQL continues through `safety/`.
- `saya-harness`: bounded workspace, process, and scratch-tool work.
- `saya-types`: only owning contracts and validated payload representations.
- Release scripts, CI, and documentation: release-specific audit corrections.

Dependency order follows the audit plan: stable config/contracts and shared error
interfaces precede their consumers; each reserved seam has one writer.

## Test list

Every slice begins with its named failing regression from the audit implementation
plan. This includes chart JSON round trips; documentation/source consistency checks;
hermetic shell fixtures; boundary conversion, file, history, endpoint, argv, SQL,
HTTP-body, streaming-fragment, cache-freshness, and persistence tests; provider and
connector contract tests; and end-to-end CLI turn-file/output-format tests. Security
and read-only safety changes retain the required property/contract tests. Each slice
runs its targeted crate gate before the workspace gate.

## Risks and rollback

The main risks are public output/schema compatibility, persisted-state migrations,
and accidental authority expansion. Additive compatibility bridges are used where
necessary; malformed or ambiguous inputs fail closed with typed, payload-free errors.
Each focused conventional commit is the rollback unit. No secrets, raw database
results, or credential-bearing release state may enter the tree.
