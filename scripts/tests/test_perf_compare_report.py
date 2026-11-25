#!/usr/bin/env python3
"""Behavior tests for the perf comparison report's steady-overhead bars."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import perf_compare_report


def record(run, *, p50, p95=None, **extra):
    base = {
        "run": run,
        "first_event_ms": 100.0,
        "overhead_p50_ms": p50,
        "overhead_p95_ms": p50 + 20.0 if p95 is None else p95,
        "request_bytes_median": 100.0,
        "idle_cpu_line_pct": 0.1,
        "idle_cpu_tui_pct": 0.1,
        "draws_per_second": 10.0,
        "frame_interval_ms": 1.0,
        "disk_bytes_per_turn": 1000.0,
        "skipped": [],
    }
    base.update(extra)
    return base


def compare(control, candidate):
    markdown, verdicts = perf_compare_report.render(
        control, candidate, "control", "candidate", len(control)
    )
    return markdown, verdicts


def verdict_for(verdicts, name):
    matches = [verdict for row, verdict in verdicts if row == name]
    if not matches:
        raise AssertionError(f"no row named {name!r}: {[row for row, _ in verdicts]}")
    return matches[0]


class SteadyOverheadBarTests(unittest.TestCase):
    def test_a_candidate_slower_than_the_paired_control_fails_below_50_ms(self):
        control = [record(run, p50=20.0) for run in range(3)]
        candidate = [record(run, p50=25.0) for run in range(3)]
        _, verdicts = compare(control, candidate)
        self.assertEqual(verdict_for(verdicts, "steady turn overhead, p50"), "FAIL")
        self.assertEqual(verdict_for(verdicts, "steady turn overhead, p95"), "FAIL")

    def test_a_candidate_at_or_below_the_paired_control_passes(self):
        control = [record(run, p50=20.0) for run in range(3)]
        candidate = [record(run, p50=18.0) for run in range(3)]
        _, verdicts = compare(control, candidate)
        self.assertEqual(verdict_for(verdicts, "steady turn overhead, p50"), "pass")
        self.assertEqual(verdict_for(verdicts, "steady turn overhead, p95"), "pass")

    def test_the_absolute_50_ms_rows_are_reported_not_asserted(self):
        control = [record(run, p50=80.0) for run in range(3)]
        candidate = [record(run, p50=90.0) for run in range(3)]
        markdown, verdicts = compare(control, candidate)
        self.assertEqual(verdict_for(verdicts, "steady turn overhead, p50"), "FAIL")
        self.assertEqual(
            verdict_for(verdicts, "steady turn overhead, p50 (absolute)"), "reported"
        )
        self.assertEqual(
            verdict_for(verdicts, "steady turn overhead, p95 (absolute)"), "reported"
        )
        self.assertIn("≤ 50 ms (reported)", markdown)

    def test_the_paired_bar_is_not_loosened_by_a_slow_absolute_reading(self):
        control = [record(run, p50=60.0) for run in range(3)]
        candidate = [record(run, p50=55.0) for run in range(3)]
        _, verdicts = compare(control, candidate)
        self.assertEqual(verdict_for(verdicts, "steady turn overhead, p50"), "pass")

    def test_the_delta_is_the_paired_difference_across_runs(self):
        control = [record(run, p50=20.0) for run in range(3)]
        candidate = [record(run, p50=25.0) for run in range(3)]
        markdown, _ = compare(control, candidate)
        self.assertIn("+5.00 ms paired over 3 run(s)", markdown)

    def test_runs_with_no_common_index_are_not_comparable(self):
        control = [record(run, p50=20.0) for run in (0, 1)]
        candidate = [record(run, p50=18.0) for run in (2, 3)]
        _, verdicts = compare(control, candidate)
        self.assertTrue(
            verdict_for(verdicts, "steady turn overhead, p50").startswith(
                "not comparable"
            )
        )


if __name__ == "__main__":
    unittest.main()
