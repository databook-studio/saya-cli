#!/usr/bin/env bash
# Compare the chat session against a control build on ref/06-plan.md §8's
# measures: run the ignored perf harness for both binaries on this machine and
# write target/perf/compare.md with the plan's pass bars applied.
#
#   scripts/perf-compare.sh <control-bin> <candidate-bin> [k]
#
# Both arms run on the same machine, one after the other, with the machine kept
# awake (caffeinate -i on macOS) so a sleeping display cannot skew a window.
# Nothing here talks to a paid provider: the harness uses the scripted mock on
# loopback. The timed campaign belongs on a quiet machine — this script says
# nothing about the numbers it collects beyond the bars the plan states.
#
# The steady-overhead bar is paired: each candidate run is compared with the
# control run of the same index from the same session, and the median
# difference must stay within 10 % of the control's median. The absolute
# ≤ 50 ms p50 and ≤ 120 ms p95 readings stay in the table as reported rows.
set -euo pipefail

usage="usage: scripts/perf-compare.sh <control-bin> <candidate-bin> [k]"
control="${1:?$usage}"
candidate="${2:?$usage}"
k="${3:-5}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
perf_dir="$root/target/perf"
mkdir -p "$perf_dir"

for bin in "$control" "$candidate"; do
    if [[ ! -x "$bin" ]]; then
        echo "perf-compare: $bin is not an executable" >&2
        exit 2
    fi
done

# The harness runs with the crate directory as its working directory, so a
# relative arm would name a file that does not exist there.
absolute() {
    (cd "$(dirname "$1")" && printf '%s/%s\n' "$(pwd)" "$(basename "$1")")
}
control="$(absolute "$control")"
candidate="$(absolute "$candidate")"

wake=()
if command -v caffeinate >/dev/null 2>&1; then
    wake=(caffeinate -i)
fi

run_arm() {
    local label="$1" bin="$2"
    echo "perf-compare: $label arm ($bin), k=$k" >&2
    rm -f "$perf_dir/$label.jsonl"
    SAYA_PERF_BIN="$bin" SAYA_PERF_K="$k" SAYA_PERF_LABEL="$label" \
        ${wake[@]+"${wake[@]}"} \
        cargo test --manifest-path "$root/Cargo.toml" -p saya-cli --locked \
        --test perf_chat -- --ignored --exact \
        the_chat_session_meets_the_plans_performance_measures --nocapture
}

run_arm control "$control"
run_arm candidate "$candidate"

python3 "$root/scripts/perf_compare_report.py" \
    "$perf_dir/control.jsonl" "$perf_dir/candidate.jsonl" \
    "$perf_dir/compare.md" "$control" "$candidate" "$k"
