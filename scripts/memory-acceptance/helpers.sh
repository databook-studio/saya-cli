# -- Run plumbing helpers ------------------------------------------------------
#
# The real-model mode's decisions, kept as functions so the self-test can hold
# them without a gateway, a database, or a built binary.

# Signatures that mean "the environment failed, not the model": an
# unauthenticated or unreachable gateway (the typed provider errors) and the
# one-shot warning that the project layer tried to set security-critical
# settings (the user layer must own them — see the config staging below).
ENV_FAILURE_PATTERN='provider request failed|provider is not configured|warning: ignored [0-9]+ security-critical'

# Scans BOTH preflight streams: ndjson renders error events to stderr, but
# scanning both cannot miss whichever stream a format change moves them to.
environment_failure_detected() {  # <stderr-file> <stdout-file> → 0 = failure found
  local stream
  for stream in "$1" "$2"; do
    if [ -s "$stream" ] && grep -Eq "$ENV_FAILURE_PATTERN" "$stream"; then
      return 0
    fi
  done
  return 1
}

# The gateway caches identical requests and replays the first answer in a
# fraction of a second; a real model call never lands that fast. A run under
# one second is therefore flagged and excluded from the sample count.
is_probable_cache_hit() {  # <seconds> → 0 = probable cache hit
  case "${1:-}" in ''|*[!0-9.]*) return 1 ;; esac
  awk -v t="$1" 'BEGIN { exit ((t + 0) < 1.0) ? 0 : 1 }'
}

# A per-run nonce appended to the prompt: identical prompts are NOT
# independent samples against a caching gateway, and a fixed suffix would
# repeat across script invocations too. The index keeps the shape readable;
# the two RANDOM draws make repeats within and across script runs improbable.
new_run_nonce() {  # <run-index>
  printf 'run-%s-%s-%s' "$1" "$RANDOM" "$RANDOM"
}

# The containers may run under podman or docker; on this Mac `docker ps` does
# not see podman's containers, so try podman first and fall back to docker.
container_running() {
  local engine
  for engine in podman docker; do
    if command -v "$engine" >/dev/null 2>&1 \
      && "$engine" ps --format '{{.Names}}' 2>/dev/null | grep -q databook-postgres
    then
      return 0
    fi
  done
  return 1
}

# --self-test: run the scorer over the fixture streams and compare each against
# the verdict encoded in its filename (`expected-<verdict>--<slug>.ndjson`).
# Needs no model, gateway, database, or built binary.
self_test() {
  local fails=0 total=0 f name expected verdict
  for f in "$FIXTURES"/expected-*.ndjson; do
    if [ ! -e "$f" ]; then
      echo "✗ no fixtures found in $FIXTURES" >&2
      return 2
    fi
    name="$(basename "$f")"
    expected="${name#expected-}"
    expected="${expected%%--*}"
    expected="$(printf '%s' "$expected" | tr '[:lower:]' '[:upper:]')"
    total=$((total + 1))
    case "$expected" in
      BOUND|OVERRIDDEN|INCONCLUSIVE) ;;
      *)
        printf 'FAIL %s — filename encodes an unknown verdict: %s\n' "$name" "$expected"
        fails=$((fails + 1))
        continue
        ;;
    esac
    verdict="$(score_stream "$f" 2>/dev/null)"
    if [ "$verdict" = "$expected" ]; then
      printf 'ok  %s → %s\n' "$name" "$verdict"
    else
      printf 'FAIL %s → got %s, expected %s\n' "$name" "$verdict" "$expected"
      fails=$((fails + 1))
    fi
  done
  echo
  if [ "$fails" -eq 0 ]; then
    printf '✓ %d/%d self-test cases pass\n' "$total" "$total"
    return 0
  fi
  printf '✗ %d/%d self-test cases fail\n' "$fails" "$total"
  return 1
}
