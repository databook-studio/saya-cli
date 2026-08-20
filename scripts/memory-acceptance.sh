#!/usr/bin/env bash
# Acceptance test: does a remembered fact actually change the SQL saya generates?
#
# This is the gate the assisted-memory work must pass before any demo is recorded.
# It is deliberately empirical: it asks the real model, through the real gateway,
# against the real pagila database, and reports the column each answer actually
# filtered on. It never inspects a prompt or a mock.
#
# Usage:
#   scripts/memory-acceptance.sh [runs-per-prompt]     # default 3
#
# Requires: .env.saya (saya reads the key itself — this script never touches its
# value), the databook-postgres container, and a built binary. Set SAYA_BIN to
# override; defaults to target/debug/saya.
#
# Exit codes: 0 = the fact bound on every run. 1 = at least one override.
set -uo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

RUNS="${1:-3}"
BIN="${SAYA_BIN:-target/debug/saya}"
OBJECT="pagila.public.rental"
CLAIMED="return_date"      # the business rule the user established
GUESS="rental_date"        # what the model picks unaided, without the contract

[ -x "$BIN" ] || { echo "✗ $BIN not built (cargo build -p saya-cli)" >&2; exit 2; }
[ -s .env.saya ] || { echo "✗ .env.saya missing or empty" >&2; exit 2; }
docker ps --format '{{.Names}}' | grep -q databook-postgres \
  || { echo "✗ databook-postgres is not running" >&2; exit 2; }

# An isolated HOME so the acceptance run never reads or writes the developer's
# real contract store. Rebuilt each run, so the result depends only on the fact
# this script establishes.
HOME_DIR="$(mktemp -d)"; trap 'rm -rf "$HOME_DIR"' EXIT
export HOME="$HOME_DIR"
export SAYA_DOCKER_POSTGRES_PASSWORD=databook SAYA_DOCKER_MYSQL_PASSWORD=databook

SAYA=("$BIN" --config .saya/config.toml --connections .saya/connections.toml --profile docker_postgres)

# The schema must be cached before the claim is made, or the claim is fingerprinted
# against nothing and lands `needs_review` instead of `current`.
"${SAYA[@]}" connection schema docker_postgres --refresh >/dev/null 2>&1 \
  || { echo "✗ schema refresh failed" >&2; exit 2; }
"${SAYA[@]}" contracts remember "$OBJECT" --kind time-column --value "$CLAIMED" >/dev/null \
  || { echo "✗ could not remember the claim" >&2; exit 2; }

# Two phrasings of one question. The second differs only by a trailing sentence,
# and that alone was enough to make the model discard a confirmed claim.
PROMPTS=(
  "How many rentals were there in each month of 2022?"
  "How many rentals were there in each month of 2022? Show me the SQL."
)

echo "claim: $OBJECT default_time_column = $CLAIMED  (confirmed, user_explicit)"
echo "runs per prompt: $RUNS"
echo

failures=0
for prompt in "${PROMPTS[@]}"; do
  echo "── \"$prompt\""
  for i in $(seq 1 "$RUNS"); do
    out="$("${SAYA[@]}" --env-file .env.saya --approval-mode read-only --non-interactive \
      ask "$prompt" 2>&1)"
    # The executed predicate, not any mention: the model names both columns while
    # explaining itself, so grepping for a bare column name reports false passes.
    used="$(printf '%s' "$out" | grep -oE "WHERE ($CLAIMED|$GUESS)" | head -1 | awk '{print $2}')"
    case "$used" in
      "$CLAIMED") echo "   run $i: bound     ($CLAIMED)" ;;
      "$GUESS")   echo "   run $i: OVERRIDDEN ($GUESS)"; failures=$((failures + 1)) ;;
      *)          echo "   run $i: inconclusive — no time predicate found"; failures=$((failures + 1)) ;;
    esac
  done
done

echo
if [ "$failures" -eq 0 ]; then
  echo "✓ the confirmed claim bound on every run"
  exit 0
fi
echo "✗ $failures run(s) did not honour the confirmed claim"
exit 1
