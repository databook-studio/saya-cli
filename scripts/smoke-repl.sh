#!/usr/bin/env bash
#
# Smoke test for the interactive REPL (Phase F interactivity features).
#
# Drives the `saya` binary with a piped slash-command script and asserts the
# expected output. Piped stdin exercises the plain-loop dispatch path, so this
# covers command parsing/dispatch, argument handling, error recovery, and the
# in-REPL SQL/session/model surfaces. It does NOT cover the reedline-only
# affordances (Tab completion menu, syntax highlighting, persistent history),
# which require an interactive TTY — see the notes in the saya-smoke skill.
#
# Hermetic: uses a throwaway session dir + state DB so it never touches your
# real ~/.local/share/saya data. Offline: only slash commands are sent, so no
# AI provider or live database is contacted (a raw /sql before selecting a
# profile deliberately hits the "no active profile" guard).
#
# Usage:  scripts/smoke-repl.sh [path-to-saya-binary]
# Exit:   0 = all assertions passed, 1 = one or more failed.

set -u

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

BIN="${1:-}"
if [[ -z "$BIN" ]]; then
  BIN="target/debug/saya"
  if [[ ! -x "$BIN" ]]; then
    echo "==> building saya (debug) ..."
    cargo build -p saya-cli || { echo "build failed"; exit 1; }
  fi
fi
if [[ ! -x "$BIN" ]]; then
  echo "saya binary not found at: $BIN"; exit 1
fi

# Hermetic, throwaway state so the run is repeatable and side-effect free.
WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT
export SAYA_SESSION_DIR="$WORKDIR/sessions"
export SAYA_STATE_DB="$WORKDIR/state.sqlite3"

echo "==> binary:      $BIN"
echo "==> session dir: $SAYA_SESSION_DIR"
echo

# The command script. Ordering matters:
#  - the /conect typo must appear BEFORE a later command whose output we assert,
#    proving a bad command no longer tears down the session.
#  - /sql runs while no profile is selected, hitting the offline guard.
OUT="$(printf '%s\n' \
  '/help' \
  '/help connect' \
  '/provider' \
  '/model' \
  '/connections' \
  '/conect prod' \
  '/sql SELECT 1' \
  '/sessions' \
  '/resume no-such-id-xyz' \
  '/exit' | "$BIN" 2>&1)"
CODE=$?

echo "----- REPL output -----"
echo "$OUT"
echo "-----------------------"
echo

fails=0
check() {  # check "<description>" "<substring that must be present>"
  if grep -qiF -- "$2" <<<"$OUT"; then
    printf '  \033[32mPASS\033[0m  %s\n' "$1"
  else
    printf '  \033[31mFAIL\033[0m  %s  (missing: %q)\n' "$1" "$2"
    fails=$((fails + 1))
  fi
}

echo "Assertions:"
check "help lists the /sql command"            "/sql <query>"
check "per-command help shows an example"       "Example: /connect"
check "bare /provider lists available providers" "available:"
check "bare /model reports the model"            "Model:"
check "did-you-mean suggests the closest command" "did you mean /connect"
# /resume runs AFTER the /conect typo, so its output appearing proves the bad
# command did not tear the session down (DB-independent, deterministic).
check "session SURVIVES a bad command (later /resume runs)" "Session not found: no-such-id-xyz"

# The session must exit cleanly via /exit (code 0), never abort mid-stream.
if [[ "$CODE" -eq 0 ]]; then
  printf '  \033[32mPASS\033[0m  clean exit (code 0)\n'
else
  printf '  \033[31mFAIL\033[0m  clean exit — got code %s\n' "$CODE"
  fails=$((fails + 1))
fi

echo
if [[ "$fails" -eq 0 ]]; then
  printf '\033[32mSMOKE OK\033[0m — all REPL assertions passed.\n'
  exit 0
else
  printf '\033[31mSMOKE FAILED\033[0m — %d assertion(s) failed.\n' "$fails"
  exit 1
fi
