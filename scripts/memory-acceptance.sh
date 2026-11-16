#!/usr/bin/env bash
# Acceptance test: does a remembered fact actually change the SQL saya generates?
#
# This is the gate the assisted-memory work must pass before any demo is recorded.
# It is deliberately empirical: it asks the real model, through the real gateway,
# against the real pagila database, and reports the column each answer actually
# filtered on. It never inspects a prompt or a mock.
#
# The oracle reads the structured `--format ndjson` stream of each run and scores
# only the SQL of queries that *executed successfully* (a tool_completed success
# paired with its tool_requested SQL). Prose never scores: a sentence naming a
# column is not evidence the query used it. No successful query is INCONCLUSIVE
# and fails closed.
#
# Usage:
#   scripts/memory-acceptance.sh [runs-per-prompt]     # default 3
#   scripts/memory-acceptance.sh --self-test           # scorer + plumbing vs
#                                                      # synthetic captures,
#                                                      # no model/db
#
# Requires (normal mode): .saya/config.toml (the intended ai settings — staged
# into the trusted user layer; see below), .env.saya (saya reads the key
# itself — this script never touches its value), the databook-postgres
# container (under podman or docker), a built binary, and python3 (the scorer
# is python stdlib). Set SAYA_BIN to override; defaults to target/debug/saya.
#
# The real-model mode reaches the configured gateway through the USER config
# layer: `--config` names the project layer, and the project layer is
# untrusted — its security-critical settings (`ai.base_url`, `ai.api_key`,
# `ai.allow_data_sharing`, `run.read_only`) are reverted with a warning, which
# is how the first real runs died with HTTP 401 before scoring anything. The
# intended file is therefore copied to `$SAYA_CONFIG_HOME/saya/config.toml`
# (the user layer, which owns those settings) and never passed as `--config`.
#
# Before any run, a preflight trivial `ask` proves the gateway actually
# authenticates; a 401/auth/transport failure — or the ignored-settings
# warning — exits 2 instead of being scored as the model ignoring the claim.
#
# Each run appends a fresh nonce to its prompt (a caching gateway replays
# identical requests in ~0.1 s, which is not an independent sample) and times
# the call: a run under one second is flagged as a probable cache hit and is
# counted neither for nor against.
#
# Exit codes: 0 = the fact bound on every independent sample (or the
# self-test passed).
# 1 = at least one independent sample overrode / was inconclusive, or a
# self-test case failed.
# 2 = the environment is not ready (missing inputs, container down, preflight
# could not reach the gateway, ignored-settings warning, or every run was a
# probable cache hit so no independent sample exists).
set -uo pipefail

ROOT="$(git rev-parse --show-toplevel)" || {
  echo "✗ not inside a git worktree (the script reads its fixtures and .saya/ from the repo root)" >&2
  exit 2
}
cd "$ROOT" || exit 2

CLAIMED="return_date"      # the business rule the user established
GUESS="rental_date"        # what the model picks unaided, without the contract
FIXTURES="scripts/testdata/memory-acceptance"

# The verdict oracle. Reads a capture of one run's ndjson stdout (the structured
# events `saya ask --format ndjson` prints) and prints BOUND, OVERRIDDEN or
# INCONCLUSIVE on stdout; a human-readable reason goes to stderr.
#
# What scores, per the audit (TA-07): only the SQL of queries that executed
# successfully. The wire carries the SQL on `tool_requested` events (the
# `detail` field: collapsed SQL, optionally suffixed ` (@connection)`), and the
# success witness on the matching `tool_completed` event — `bounded_sql_query`
# completes as "read-only database tool completed" and fails with a summary
# keeping the substring "failed" (denied calls emit `tool_denied` and never
# complete). Each completion is paired with the oldest unanswered request of
# the same tool, so a failed or denied execution leaves its SQL unwitnessed and
# prose can never score. `answer_designated` / `consensus_decided` name the
# answering query; when their SQL is in the witnessed set, it alone decides.
# python3 stdlib only, by design: no jq dependency, no grep over prose.

source "$ROOT/scripts/memory-acceptance/scorer.sh"
source "$ROOT/scripts/memory-acceptance/helpers.sh"
source "$ROOT/scripts/memory-acceptance/plumbing.sh"

if [ "${1:-}" = "--self-test" ]; then
  self_test
  fixture_status=$?
  plumbing_test
  plumbing_status=$?
  if [ "$fixture_status" -ne 0 ]; then exit "$fixture_status"; fi
  exit "$plumbing_status"
fi

RUNS="${1:-3}"
case "$RUNS" in
  ''|*[!0-9]*) { echo "✗ runs-per-prompt must be a number (got: $RUNS)" >&2; exit 2; } ;;
esac
if [ "$RUNS" -lt 1 ]; then
  echo "✗ runs-per-prompt must be at least 1 (a zero-run experiment passes vacuously and proves nothing)" >&2
  exit 2
fi
BIN="${SAYA_BIN:-target/debug/saya}"
OBJECT="pagila.public.rental"

[ -x "$BIN" ] || { echo "✗ $BIN not built (cargo build -p saya-cli)" >&2; exit 2; }
[ -s .env.saya ] || { echo "✗ .env.saya missing or empty" >&2; exit 2; }
[ -s .saya/config.toml ] \
  || { echo "✗ .saya/config.toml missing or empty (the intended ai settings live there)" >&2; exit 2; }
command -v python3 >/dev/null 2>&1 || { echo "✗ python3 not found (the scorer needs it)" >&2; exit 2; }
container_running \
  || { echo "✗ databook-postgres is not running (checked podman, then docker)" >&2; exit 2; }

# An isolated HOME so the acceptance run never reads or writes the developer's
# real contract store. Rebuilt each run, so the result depends only on the fact
# this script establishes.
HOME_DIR="$(mktemp -d)"
STREAM="$(mktemp)"; STDERRS="$(mktemp)"; REASON="$(mktemp)"
PREFLIGHT_OUT="$(mktemp)"; PREFLIGHT_ERR="$(mktemp)"
trap 'rm -rf "$HOME_DIR" "$STREAM" "$STDERRS" "$REASON" "$PREFLIGHT_OUT" "$PREFLIGHT_ERR"' EXIT
export HOME="$HOME_DIR"
export SAYA_DOCKER_POSTGRES_PASSWORD=databook SAYA_DOCKER_MYSQL_PASSWORD=databook

# The intended ai settings live in the project layer, which saya does not
# trust: passed as `--config` (or discovered from the cwd), their security-
# critical fields are reverted with a warning. They belong in the USER layer,
# which owns them — staged under SAYA_CONFIG_HOME, the user config dir. The
# preflight below refuses to run if the warning still appears.
export SAYA_CONFIG_HOME="$HOME_DIR/config"
mkdir -p "$SAYA_CONFIG_HOME/saya" \
  || { echo "✗ could not create the user config dir" >&2; exit 2; }
cp .saya/config.toml "$SAYA_CONFIG_HOME/saya/config.toml" \
  || { echo "✗ could not stage .saya/config.toml into the user layer" >&2; exit 2; }

SAYA=("$BIN" --connections .saya/connections.toml --profile docker_postgres)

# The preflight proves the run can actually reach the configured gateway: one
# trivial ask. `config doctor` cannot prove this — it performs no network
# checks — so the proof is a real request. An environment failure here exits 2
# before anything is scored; that is the difference between "the model ignored
# the claim" and "the environment was never ready".
"${SAYA[@]}" --env-file .env.saya --approval-mode read-only --non-interactive \
  --format ndjson ask "Connectivity check: reply with the single word OK and nothing else." \
  >"$PREFLIGHT_OUT" 2>"$PREFLIGHT_ERR"
preflight_status=$?
if environment_failure_detected "$PREFLIGHT_ERR" "$PREFLIGHT_OUT"; then
  detail="$(grep -Ehm1 "$ENV_FAILURE_PATTERN" "$PREFLIGHT_ERR" "$PREFLIGHT_OUT" 2>/dev/null \
    | cut -c1-160 | sed -n '1p')"
  echo "✗ environment not ready: ${detail:-the preflight request failed}" >&2
  exit 2
fi
if [ "$preflight_status" -ne 0 ]; then
  echo "✗ environment not ready: the preflight ask exited $preflight_status" >&2
  [ -s "$PREFLIGHT_ERR" ] \
    && tail -n 3 "$PREFLIGHT_ERR" | cut -c1-160 | sed 's/^/     /' >&2
  exit 2
fi

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

# Per run: a fresh nonce defeats the gateway's request cache, and the wall
# time tells an independent call from a replayed one. A probable cache hit is
# reported but counted neither for nor against — it is not an independent
# sample, so it cannot prove or break the claim.
samples=0
sample_failures=0
cache_hits=0
for prompt in "${PROMPTS[@]}"; do
  echo "── \"$prompt\""
  for i in $(seq 1 "$RUNS"); do
    : > "$STREAM"; : > "$STDERRS"; : > "$REASON"
    nonce="$(new_run_nonce "$i")"
    started="$(python3 -c 'import time; print(time.monotonic())')"
    "${SAYA[@]}" --env-file .env.saya --approval-mode read-only --non-interactive \
      --format ndjson ask "$prompt ($nonce)" >"$STREAM" 2>"$STDERRS"
    ended="$(python3 -c 'import time; print(time.monotonic())')"
    elapsed="$(awk -v a="$started" -v b="$ended" 'BEGIN{printf "%.2f", b - a}')"
    # The verdict comes only from the structured stream: executed SQL with a
    # successful completion. Prose, denials, and failed executions never score.
    verdict="$(score_stream "$STREAM" 2>"$REASON")"
    reason="$(cat "$REASON")"
    if is_probable_cache_hit "$elapsed"; then
      cache_hits=$((cache_hits + 1))
      cached=yes
      timing="[$elapsed s — probable cache hit, not counted as a sample]"
    else
      samples=$((samples + 1))
      cached=no
      timing="[$elapsed s]"
    fi
    case "$verdict" in
      BOUND) echo "   run $i: bound     ($CLAIMED) $timing" ;;
      OVERRIDDEN)
        echo "   run $i: OVERRIDDEN ($GUESS) $timing"
        # A cache-hit run is not an independent sample: neither evidence for
        # nor against.
        [ "$cached" = no ] && sample_failures=$((sample_failures + 1))
        ;;
      *)
        echo "   run $i: inconclusive — ${reason:-no verdict} $timing"
        if [ -s "$STDERRS" ]; then
          tail -n 1 "$STDERRS" | cut -c1-160 | sed 's/^/     stderr: /'
        fi
        [ "$cached" = no ] && sample_failures=$((sample_failures + 1))
        ;;
    esac
  done
done

echo
if [ "$samples" -eq 0 ] && [ "$cache_hits" -gt 0 ]; then
  echo "✗ environment not ready: all $cache_hits run(s) were probable cache hits (under 1 s);" >&2
  echo "  no independent sample reached the model — the gateway may be replaying identical requests." >&2
  exit 2
fi
if [ "$cache_hits" -gt 0 ]; then
  excluded=" ($cache_hits probable cache hit(s) excluded from scoring)"
else
  excluded=""
fi
if [ "$sample_failures" -eq 0 ]; then
  echo "✓ the confirmed claim bound on all $samples independent sample(s)$excluded"
  exit 0
fi
echo "✗ $sample_failures of $samples independent sample(s) did not honour the confirmed claim$excluded"
exit 1