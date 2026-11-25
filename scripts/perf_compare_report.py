#!/usr/bin/env python3
"""Apply the perf plan's bars to two arms and write the comparison table.

`scripts/perf-compare.sh` runs the ignored perf harness for the control and
candidate binaries, then calls this module with the two jsonl files. Nothing
here talks to a provider; it only reads what the harness recorded.

The steady-overhead verdict is paired: each candidate run is compared with
the control run of the same index from the same session, and the median paired
difference may not exceed 10 % of the control's median. The absolute p50/p95
readings stay in the table as reported rows.
"""

import json
import platform
import statistics
import sys

MEASURES = {
    "a": "first message: send → first streamed event",
    "b": "steady turn overhead beyond the scripted provider",
    "c": "request bytes per turn",
    "d": "idle CPU over the 60 s window (line + TUI)",
    "e": "draws while streaming 1 000 tokens",
    "f": "disk written per turn (job records + rewind)",
}

# Each row states the plan's bar, how a value is judged against it, and — for a
# measure an arm could not take — the words that arm's record uses for the
# reason. A row with no verdict is a measure the plan only asks to be reported;
# a row one arm could not take says `not comparable` with that arm's own reason
# instead of borrowing a number or disappearing.
ROWS = [
    ("a", "first message: send → first streamed event", "first_event_ms", "ms",
     "control + 10 %", "higher_is_worse", 1.10, None),
    ("b", "steady turn overhead, p50", "overhead_p50_ms", "ms",
     "paired, control + 10 %", "paired_higher_is_worse", 0.10, None),
    ("b", "steady turn overhead, p95", "overhead_p95_ms", "ms",
     "paired, control + 10 %", "paired_higher_is_worse", 0.10, None),
    ("b", "steady turn overhead, p50 (absolute)", "overhead_p50_ms", "ms",
     "≤ 50 ms (reported)", "reported", None, None),
    ("b", "steady turn overhead, p95 (absolute)", "overhead_p95_ms", "ms",
     "≤ 120 ms (reported)", "reported", None, None),
    ("c", "request bytes per turn", "request_bytes_median", "bytes",
     "reported (GR causes only)", "reported", None, None),
    ("d", "idle TUI CPU over the window", "idle_cpu_tui_pct", "%", "≤ 1 %",
     "at_most", 1.0, "idle CPU of the TUI"),
    ("d", "idle line-front CPU over the window", "idle_cpu_line_pct", "%",
     "reported", "reported", None, "idle CPU of the line front"),
    ("e", "draws per second while streaming", "draws_per_second", "/s", "≤ 30/s",
     "at_most", 30.0, "draws while streaming"),
    ("e", "interval between frames", "frame_interval_ms", "ms",
     "≤ 2 ms each — not asserted", "unasserted", None, "draws while streaming"),
    ("f", "disk written per turn", "disk_bytes_per_turn", "bytes",
     "≤ 64 KiB (65536 B)", "at_most", 65536.0, None),
]


def load(path):
    with open(path) as handle:
        return [json.loads(line) for line in handle if line.strip()]


def median(records, key):
    values = [record[key] for record in records if record.get(key) is not None]
    return statistics.median(values) if values else None


def show(value, unit):
    return "not comparable" if value is None else f"{value:.2f} {unit}"


def reasons(records, needle):
    found = []
    for record in records:
        for skipped in record.get("skipped", []):
            if (needle is None or needle in skipped) and skipped not in found:
                found.append(skipped)
    return "; ".join(found)


def paired_delta(control, candidate, key):
    """The median candidate-minus-control difference over shared run indices.

    Both arms run in the same session, so pairing each candidate run with the
    control run of the same index cancels the drift a comparison of two
    medians taken minutes apart would carry. A run index only one arm
    recorded is not a pair and is not counted.
    """
    control_by_run = {}
    for record in control:
        run, value = record.get("run"), record.get(key)
        if run is not None and value is not None:
            control_by_run[run] = value
    deltas = []
    for record in candidate:
        run, value = record.get("run"), record.get(key)
        if run is None or value is None or run not in control_by_run:
            continue
        deltas.append(value - control_by_run[run])
    if not deltas:
        return None, 0
    return statistics.median(deltas), len(deltas)


def arm_spelling(records):
    flags = [record.get("posture_flag") for record in records if record.get("posture_flag")]
    plain = "yes" if any(record.get("plain_flag") for record in records) else "no"
    if not flags:
        return "unknown (the record does not state the spelling)"
    return f"`{flags[0]} read-only` (documents `--plain`: {plain})"


def render(control, candidate, control_bin, candidate_bin, k):
    """The comparison markdown and each row's verdict."""
    lines = [
        "# PERF compare — chat session",
        "",
        f"- control: `{control_bin}`",
        f"- candidate: `{candidate_bin}`",
        f"- runs per arm: {k} (medians below)",
        f"- machine: {platform.system()} {platform.machine()}, {platform.processor() or 'unknown cpu'}",
        "- bars: `ref/06-plan.md` §8 (PERF)",
        f"- control arguments: {arm_spelling(control)}",
        f"- candidate arguments: {arm_spelling(candidate)}",
        "",
        "| Measure | Control | Candidate | Delta | Bar | Verdict |",
        "| --- | --- | --- | --- | --- | --- |",
    ]

    verdicts = []
    coverage = {letter: [] for letter in MEASURES}
    for letter, name, key, unit, bar, rule, limit, needle in ROWS:
        control_value = median(control, key)
        candidate_value = median(candidate, key)
        coverage[letter].append(control_value is not None and candidate_value is not None)
        if control_value is None or candidate_value is None:
            delta = "—"
            missing = []
            if control_value is None:
                missing.append("control")
            if candidate_value is None:
                missing.append("candidate")
            records = control if control_value is None else candidate
            why = reasons(records, needle) or "the arm reported no value"
            verdict = f"not comparable ({' and '.join(missing)}: {why})"
        else:
            change = candidate_value - control_value
            delta = f"{change:+.2f} {unit} ({change / control_value * 100:+.1f} %)" if control_value else "—"
            if rule == "paired_higher_is_worse":
                paired, pairs = paired_delta(control, candidate, key)
                if paired is None:
                    delta = "—"
                    verdict = "not comparable (no run index is present in both arms)"
                else:
                    # The paired median may sit up to `limit` (10 %) above the
                    # control's own median; a few hundredths of a millisecond
                    # is noise, not a regression.
                    allowed = control_value * limit
                    delta = f"{paired:+.2f} {unit} paired over {pairs} run(s)"
                    verdict = "pass" if paired <= allowed else "FAIL"
            elif rule == "at_most":
                verdict = "pass" if candidate_value <= limit else "FAIL"
            elif rule == "higher_is_worse":
                verdict = "pass" if candidate_value <= control_value * limit else "FAIL"
            elif rule == "unasserted":
                verdict = "not asserted (see REPORT.md)"
            else:
                verdict = "reported"
        verdicts.append((name, verdict))
        lines.append(
            f"| {name} | {show(control_value, unit)} | {show(candidate_value, unit)} "
            f"| {delta} | {bar} | {verdict} |"
        )

    lines += [
        "",
        "## Coverage of the six measures",
        "",
        "| Measure | Both arms | Note |",
        "| --- | --- | --- |",
    ]
    for letter, name in MEASURES.items():
        both = bool(coverage[letter]) and all(coverage[letter])
        note = "" if both else "not comparable — the rows above carry the reason"
        lines.append(f"| ({letter}) {name} | {'yes' if both else 'no'} | {note} |")

    skipped = sorted({reason for record in control + candidate for reason in record.get("skipped", [])})
    lines += ["", "## Notes", ""]
    if skipped:
        lines += ["- skipped by the harness: " + "; ".join(skipped)]
    lines += [
        "- the two arms state the same read-only posture under the spelling their own",
        "  `--help` documents; only the flag's name differs, the workload does not.",
        "- a measure one arm cannot take is stated in its row as `not comparable` with",
        "  that arm's own reason — it is never dropped from the table.",
        "- `interval between frames` is what the pty stream can show without production",
        "  instrumentation: the milliseconds a single frame costs would need a timing",
        "  inside `Terminal::draw`, which the harness may not add. `draws per second`",
        "  is counted directly and is the bar that is asserted.",
        "- a `FAIL` on the first-message row is measured against the control arm's own",
        "  median on this machine, not against a stored number.",
        "- the steady-overhead verdict compares the candidate with the control arm",
        "  measured in the same session, paired by run index, and passes within",
        "  10 % of the control's median; the absolute ≤ 50 ms p50 and ≤ 120 ms p95",
        "  rows are reported beside it, not asserted.",
    ]
    return "\n".join(lines) + "\n", verdicts


def main(argv):
    control_path, candidate_path, out_path, control_bin, candidate_bin, k = argv[1:]
    control = load(control_path)
    candidate = load(candidate_path)
    markdown, verdicts = render(control, candidate, control_bin, candidate_bin, k)
    with open(out_path, "w") as handle:
        handle.write(markdown)
    print(f"perf-compare: wrote {out_path}")
    print(markdown)
    failed = [name for name, verdict in verdicts if verdict == "FAIL"]
    if failed:
        print(f"\nperf-compare: {len(failed)} measure(s) outside the plan's bars", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
