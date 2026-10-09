#!/usr/bin/env bash
# scripts/walkthrough.sh — a recorded, reproducible synthetic walkthrough of
# the whole journey: demo → setup refusal → question → inspect SQL/evidence →
# save → export → import in a second environment → rerun, then the 0.4.2
# surface on top: a parameterised investigation (saved, run, and refused when
# the required parameter is missing), remembered context exported to the
# second environment and queued there pending review, a CSV staged by
# `saya open` as a read-only text snapshot (previewed, listed, queried,
# cleaned up), and the same bounded surface spoken to over MCP stdio JSON-RPC.
#
#   bash scripts/walkthrough.sh                 # uses `cargo run -q -p saya-cli --`
#   SAYA_BIN=/path/to/saya scripts/walkthrough.sh
#   WALKTHROUGH_KEEP=1 bash scripts/walkthrough.sh   # keep the sandbox for inspection
#
# Everything runs against SYNTHETIC data only, with no AI provider and no
# network: the demo database is a local SQLite file and every command below
# is the bounded, read-only path. Two isolated roots ("alice" and "bob") each
# get their own HOME, SAYA_CONFIG_HOME, SAYA_STATE_DB, SAYA_DEMO_DIR, and
# SAYA_INVESTIGATIONS_DIR, so the export/import leg really crosses between
# two environments. Every command is printed before its output, every exit
# code is checked against the expected one, and any unexpected result exits
# non-zero. docs/walkthrough.md embeds the recorded output of this script.

set -u -o pipefail

cd "$(dirname "$0")/.."

SAYA_DISPLAY="cargo run -q -p saya-cli --"
SAYA_PROG=(cargo run -q -p saya-cli --)
if [ -n "${SAYA_BIN:-}" ]; then
  SAYA_DISPLAY="$SAYA_BIN"
  SAYA_PROG=("$SAYA_BIN")
fi

ROOT=$(mktemp -d "${TMPDIR:-/tmp}/saya-walkthrough.XXXXXX")
ALICE_HOME="$ROOT/alice"
BOB_HOME="$ROOT/bob"
ALICE_CON="$ALICE_HOME/demo/connections.toml"
BOB_CON="$BOB_HOME/demo/connections.toml"
SHARED="$ROOT/shared"

if [ -z "${WALKTHROUGH_KEEP:-}" ]; then
  trap 'rm -rf "$ROOT"' EXIT
fi
mkdir -p "$SHARED"

# Nothing from the outer environment may steer this walkthrough.
unset SAYA_PROFILE SAYA_DB_TYPE SAYA_TRUST_PROJECT_CONFIG SAYA_EXTRACTION_TRACE

# Hijacking HOME below would send rustup and cargo to the empty sandbox, so
# pin their homes to the real one unless the environment already chose them.
REAL_HOME="$HOME"
RUSTUP_HOME="${RUSTUP_HOME:-$REAL_HOME/.rustup}"
CARGO_HOME="${CARGO_HOME:-$REAL_HOME/.cargo}"
export RUSTUP_HOME CARGO_HOME

LAST_OUT=""


source "$REPO_DIR/scripts/walkthrough/helpers.sh"
source "$REPO_DIR/scripts/walkthrough/journey.sh"
source "$REPO_DIR/scripts/walkthrough/mcp.sh"
