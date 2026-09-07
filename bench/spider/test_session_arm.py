#!/usr/bin/env python3
"""Tests for bench.py's session arm — one session per database.

Run with:  python3 bench/spider/test_session_arm.py
"""
import contextlib, io, json, os, shutil, stat, sys, tempfile, unittest
from unittest import mock

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.abspath(os.path.join(_HERE, "..", ".."))

import importlib.util
_spec = importlib.util.spec_from_file_location("bench_under_test",
                                               os.path.join(_HERE, "bench.py"))
bench = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(bench)

BIN = "/dev/null/fake-saya"

# A completed turn, as the real REPL writes it: a session with one turn.
_SAVED = ('{"id":"s-stub","version":1,"turns":[{"user":"u","assistant":"a",'
          '"database_derived":true}]}')
# An errored turn: the REPL still saves, but with no turns.
_EMPTY = '{"id":"s-err","version":1,"turns":[]}'


class TempBench(unittest.TestCase):
    """Isolated corpus/results directories and a stub saya binary."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="saya-bench-test-")
        self.old = {k: os.environ.get(k) for k in
                    ("SAYA_BENCH_BIN", "SAYA_BENCH_ARM", "SAYA_AI_API_KEY",
                     "FAKE_SAYA_LOG")}
        self.old_attrs = {name: getattr(bench, name) for name in
                          ("OUT", "RESULTS", "SCORED", "ARM", "CORPUS", "HOME")}
        self.out = os.path.join(self.tmp, "results")
        os.makedirs(self.out, exist_ok=True)
        bench.OUT = self.out
        bench.CORPUS = os.path.join(self.tmp, "corpus")
        bench.HOME = os.path.join(self.tmp, "home")
        os.makedirs(bench.CORPUS, exist_ok=True)
        os.makedirs(bench.HOME, exist_ok=True)
        os.environ["SAYA_BENCH_ARM"] = "default"
        bench.ARM = "default"
        bench.RESULTS = os.path.join(self.out, "results-default.ndjson")
        bench.SCORED = os.path.join(self.out, "scored-default.ndjson")

    def tearDown(self):
        for k in ("SAYA_BENCH_BIN", "SAYA_BENCH_ARM", "SAYA_AI_API_KEY",
                  "FAKE_SAYA_LOG"):
            os.environ.pop(k, None)
        for name, value in self.old_attrs.items():
            setattr(bench, name, value)
        shutil.rmtree(self.tmp, ignore_errors=True)

    def install_stub(self, fail_on_continue=False, fail_first_question=False,
                 slow=False):
        """A fake saya binary: logs argv and stdin, emits a minimal turn.

        Like the real REPL it saves a session with a recorded turn after a
        completed turn and a turns-less session after an errored one, so the
        continuation probe sees what the real store would hold.
        """
        path = os.path.join(self.tmp, "fake-saya.sh")
        log = os.path.join(self.tmp, "calls.ndjson")
        open(log, "w").close()
        os.environ["FAKE_SAYA_LOG"] = log
        os.environ["SAYA_BENCH_BIN"] = path
        fail = "1" if fail_on_continue else "0"
        fail_first = "1" if fail_first_question else "0"
        slow = "sleep 2" if slow else ""
        with open(path, "w") as f:
            f.write(f"""#!/bin/sh
LOG="$FAKE_SAYA_LOG"
for arg in "$@"; do printf '%s\\n' "$arg" >> "$LOG"; done
printf -- '--ARGS-EOF--\\n' >> "$LOG"
IN="$(cat)"
printf '%s\\n' "$IN" >> "$LOG"
printf -- '--STDIN-EOF--\\n' >> "$LOG"
mkdir -p "$SAYA_SESSION_DIR"
touch "$SAYA_STATE_DB"
{slow}
case "$IN" in *"question 0"*)
  if [ "{fail_first}" = "1" ]; then
    printf '{_EMPTY}\\n' > "$SAYA_SESSION_DIR/errored.json"
    echo '{{"event":"error","message":"gateway down"}}'
    exit 0
  fi
  ;;
esac
case " $* " in *" --continue "*)
  if [ "{fail}" = "1" ]; then
    echo "Error: requested session was not found" >&2
    exit 2
  fi
  ;;
esac
printf '{_SAVED}\\n' > "$SAYA_SESSION_DIR/saved.json"
cat <<'EOF'
{{"event":"tool_requested","name":"bounded_sql_query","detail":"SELECT 1"}}
{{"event":"tool_completed","name":"bounded_sql_query","summary":"completed 1 row"}}
{{"event":"answer_designated","sql":"SELECT 1"}}
{{"event":"complete"}}
EOF
""")
        os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)
        return path

    def calls(self):
        """The stub's recorded invocations: argv list + stdin text, in order."""
        out, cur, section = [], {"argv": [], "stdin": ""}, "argv"
        for line in open(os.environ["FAKE_SAYA_LOG"]):
            line = line.rstrip("\n")
            if line == "--ARGS-EOF--":
                section = "stdin"
            elif line == "--STDIN-EOF--":
                out.append(cur)
                cur, section = {"argv": [], "stdin": ""}, "argv"
            elif section == "argv":
                cur["argv"].append(line)
            else:
                cur["stdin"] += line + "\n"
        return out

    def items(self, n=3):
        out = []
        for i in range(n):
            it = {"_key": f"v1:{i:05d}", "question": f"question {i}",
                  "db_id": "dbtest", "query": "SELECT 1", "_doc": ""}
            if i == 0:
                it["_doc"] = "line one\nline two\nline three"
            out.append(it)
        return out

    def run_unit(self, items=None, session=False, done=None, **kw):
        return bench.run_unit(("v1", "dbtest", self.items() if items is None else items),
                              done or set(), 600, {}, 1,
                              session=session, results=bench.results_path(session),
                              **kw)

    def rows(self, session):
        return [json.loads(l) for l in open(bench.results_path(session))]


class TestPaths(TempBench):
    def test_default_paths_unchanged(self):
        self.assertEqual(bench.results_path(False),
                         os.path.join(self.out, "results-default.ndjson"))
        self.assertEqual(bench.scored_path(False),
                         os.path.join(self.out, "scored-default.ndjson"))

    def test_session_paths_are_suffixed(self):
        self.assertEqual(bench.results_path(True),
                         os.path.join(self.out, "results-default-session.ndjson"))
        self.assertEqual(bench.scored_path(True),
                         os.path.join(self.out, "scored-default-session.ndjson"))

    def test_session_paths_use_the_arm_name(self):
        bench.ARM = "hotfix"
        self.assertEqual(bench.results_path(True),
                         os.path.join(self.out, "results-hotfix-session.ndjson"))


class TestPrompt(TempBench):
    def test_multiline_prompt_folds_to_one_line(self):
        folded = bench.session_prompt({"question": "how many?",
                                       "_doc": "a\nb\r\n  c"})
        self.assertNotIn("\n", folded)
        self.assertIn("how many? --- Reference documentation", folded)
        self.assertTrue(folded.endswith("a b c"))

    def test_prompt_without_doc_passes_through(self):
        self.assertEqual(bench.session_prompt({"question": "count rows",
                                               "_doc": ""}), "count rows")

    def test_slash_leading_prompt_is_refused(self):
        with self.assertRaises(ValueError):
            bench.session_prompt({"question": "/clear", "_doc": ""})

    def test_whitespace_then_slash_is_refused_after_folding(self):
        with self.assertRaises(ValueError):
            bench.session_prompt({"question": "  /clear", "_doc": ""})

    def test_mid_string_slash_is_a_question(self):
        folded = bench.session_prompt({"question": "explain /sql usage",
                                       "_doc": ""})
        self.assertEqual(folded, "explain /sql usage")

    def test_empty_question_is_refused(self):
        with self.assertRaises(ValueError):
            bench.session_prompt({"question": "  \n ", "_doc": ""})


class TestSessionHasTurns(TempBench):
    def test_missing_dir_is_false(self):
        self.assertFalse(bench.session_has_turns(os.path.join(self.tmp, "nope")))

    def test_empty_dir_is_false(self):
        d = os.path.join(self.tmp, "sessions")
        os.makedirs(d)
        self.assertFalse(bench.session_has_turns(d))

    def test_session_with_turns_is_true(self):
        d = os.path.join(self.tmp, "sessions")
        os.makedirs(d)
        json.dump({"id": "s1", "turns": [{"user": "q", "assistant": "a"}]},
                  open(os.path.join(d, "s1.json"), "w"))
        self.assertTrue(bench.session_has_turns(d))

    def test_session_without_turns_is_false(self):
        d = os.path.join(self.tmp, "sessions")
        os.makedirs(d)
        json.dump({"id": "s1", "turns": []},
                  open(os.path.join(d, "s1.json"), "w"))
        self.assertFalse(bench.session_has_turns(d))

    def test_corrupt_session_is_false(self):
        d = os.path.join(self.tmp, "sessions")
        os.makedirs(d)
        open(os.path.join(d, "s1.json"), "w").write("{not json")
        self.assertFalse(bench.session_has_turns(d))

    def test_temp_files_are_ignored(self):
        d = os.path.join(self.tmp, "sessions")
        os.makedirs(d)
        shutil.copy(self.install_stub(), os.path.join(d, "x.json.tmp"))
        self.assertFalse(bench.session_has_turns(d))

    def test_newest_session_decides_not_the_oldest(self):
        d = os.path.join(self.tmp, "sessions")
        os.makedirs(d)
        old, new = os.path.join(d, "old.json"), os.path.join(d, "new.json")
        json.dump({"id": "old", "turns": [{"user": "q", "assistant": "a"}]},
                  open(old, "w"))
        json.dump({"id": "new", "turns": []}, open(new, "w"))
        os.utime(old, (1, 1))
        os.utime(new, (2, 2))
        self.assertFalse(bench.session_has_turns(d))
        os.utime(old, (3, 3))
        os.utime(new, (2, 2))
        self.assertTrue(bench.session_has_turns(d))


class TestCommands(TempBench):
    def test_default_argv_is_pinned(self):
        """The independent arm must build exactly the pre-change command."""
        cmd = bench.question_command(
            BIN, "q", ["cfg.toml", "conn.toml"], "prof", [], 1)
        self.assertEqual(cmd,
                         [BIN, "ask", "--config", "cfg.toml",
                          "--connections", "conn.toml", "--profile", "prof",
                          "--non-interactive", "--approval-mode", "read-only",
                          "--format", "ndjson", "q"])

    def test_session_command_is_the_bare_repl(self):
        """No subcommand, no --non-interactive — the session surface."""
        cmd = bench.session_command(
            BIN, ["cfg.toml", "conn.toml"], "prof", [], continued=False)
        self.assertEqual(cmd,
                         [BIN, "--config", "cfg.toml",
                          "--connections", "conn.toml", "--profile", "prof",
                          "--approval-mode", "read-only", "--format", "ndjson"])
        self.assertNotIn("--non-interactive", cmd)

    def test_session_command_continues_only_when_asked(self):
        cmd = bench.session_command(
            BIN, ["cfg.toml", "conn.toml"], "prof", ["extra"], continued=True)
        self.assertIn("--continue", cmd)
        self.assertEqual(cmd[-3:-1], ["--include-profile", "extra"])
        self.assertEqual(cmd[-1], "--continue")


class TestRunUnitSession(TempBench):
    def test_first_call_has_no_continue_later_calls_continue(self):
        self.install_stub()
        self.assertEqual(self.run_unit(session=True), 3)
        calls = self.calls()
        self.assertEqual(len(calls), 3)
        self.assertNotIn("--continue", calls[0]["argv"])
        self.assertIn("--continue", calls[1]["argv"])
        self.assertIn("--continue", calls[2]["argv"])

    def test_session_spawns_are_never_ask(self):
        """The whole arm is worthless if a spawn drifts back to `saya ask`:
        ask runs with empty history and never touches the session store, so
        the rows would measure the independent arm while labelled session."""
        self.install_stub()
        self.run_unit(session=True)
        for call in self.calls():
            self.assertNotIn("ask", call["argv"])

    def test_session_rows_carry_mode_and_continuity(self):
        self.install_stub()
        self.run_unit(session=True)
        rows = self.rows(True)
        self.assertEqual([r["mode"] for r in rows], ["session"] * 3)
        self.assertEqual([r["session_continued"] for r in rows],
                         [False, True, True])

    def test_session_rows_pipe_the_folded_question_on_stdin(self):
        self.install_stub()
        self.run_unit(session=True)
        calls = self.calls()
        self.assertEqual(calls[0]["stdin"].strip(),
                         "question 0 --- Reference documentation supplied "
                         "with this question --- line one line two line three")
        self.assertEqual(calls[1]["stdin"].strip(), "question 1")

    def test_session_writes_the_suffixed_results_file_only(self):
        self.install_stub()
        self.run_unit(session=True)
        self.assertTrue(os.path.exists(bench.results_path(True)))
        self.assertFalse(os.path.exists(bench.results_path(False)))

    def test_session_mode_without_a_results_file_is_refused(self):
        self.install_stub()
        with self.assertRaises(ValueError):
            bench.run_unit(("v1", "dbtest", self.items()), set(), 600, {}, 1,
                           session=True, results=None)
        self.assertEqual(self.calls(), [])

    def test_default_rows_are_labelled_independent(self):
        self.install_stub()
        self.run_unit(session=False)
        rows = self.rows(False)
        self.assertEqual([r["mode"] for r in rows], ["independent"] * 3)
        self.assertNotIn("session_continued", rows[0])

    def test_default_mode_never_pipes_stdin(self):
        self.install_stub()
        self.run_unit(session=False)
        self.assertFalse(self.calls()[0]["stdin"].strip())

    def test_resume_mid_database_continues_the_session(self):
        self.install_stub()
        items = self.items()
        with open(bench.results_path(True), "w") as f:
            f.write(json.dumps({"suite": "v1", "db": "dbtest",
                                "key": items[0]["_key"], "mode": "session"}) + "\n")
        sessions = os.path.join(self.out, "default-session", "v1", "dbtest",
                                "sessions")
        os.makedirs(sessions)
        json.dump({"id": "s1", "turns": [{"user": "earlier", "assistant": "a"}]},
                  open(os.path.join(sessions, "s1.json"), "w"))
        bench.run_unit(("v1", "dbtest", items),
                       {items[0]["_key"]}, 600, {}, 1,
                       session=True, results=bench.results_path(True))
        calls = self.calls()
        self.assertEqual(len(calls), 2)          # the first key is skipped
        self.assertIn("--continue", calls[0]["argv"])
        self.assertTrue(self.rows(True)[-1]["session_continued"])

    def test_session_not_found_falls_back_and_says_so(self):
        self.install_stub(fail_on_continue=True)
        self.assertEqual(self.run_unit(session=True), 3)
        self.assertEqual([r["session_continued"] for r in self.rows(True)],
                         [False, False, False])

    def test_errored_turns_do_not_claim_continuity(self):
        """A failed turn saves a session with no turns. The next question must
        spawn fresh and say `session_continued: false` — counting attempts
        instead of turns would label an empty conversation as continuity."""
        self.install_stub(fail_first_question=True)
        with mock.patch.object(bench.time, "sleep", lambda _s: None):
            self.assertEqual(self.run_unit(session=True), 3)
        calls = self.calls()
        self.assertEqual(len(calls), 5)      # question 0 burned its 3 attempts
        self.assertNotIn("--continue", calls[-2]["argv"])
        self.assertIn("--continue", calls[-1]["argv"])
        self.assertEqual([r["session_continued"] for r in self.rows(True)],
                         [False, False, True])

    def test_timeout_keeps_the_stream_and_does_not_fabricate_continuity(self):
        self.install_stub(slow=True)
        bench.run_unit(("v1", "dbtest", self.items(2)), set(), 1, {}, 1,
                       session=True, results=bench.results_path(True))
        rows = self.rows(True)
        self.assertTrue(rows[0]["timed_out"])
        self.assertFalse(rows[1]["session_continued"])
        raw = os.path.join(self.out, "default-session", "v1", "dbtest",
                           "00000.ndjson")
        self.assertTrue(os.path.exists(raw))

    def test_session_raw_streams_are_kept_per_question(self):
        self.install_stub()
        self.run_unit(session=True)
        raw = os.path.join(self.out, "default-session", "v1", "dbtest",
                           "00000.ndjson")
        self.assertTrue(os.path.exists(raw))
        self.assertIn('"event":"answer_designated"', open(raw).read())

    def test_session_state_and_session_dirs_live_under_the_suffixed_work(self):
        self.install_stub()
        self.run_unit(session=True)
        base = os.path.join(self.out, "default-session", "v1", "dbtest")
        self.assertTrue(os.path.exists(os.path.join(base, "state.db")))
        self.assertTrue(os.path.isdir(os.path.join(base, "sessions")))

    def test_todo_order_is_the_corpus_order(self):
        self.install_stub()
        items = list(reversed(self.items()))
        self.run_unit(items, session=True)
        calls = self.calls()
        self.assertEqual(calls[1]["stdin"].strip(), "question 1")
        self.assertEqual(calls[0]["stdin"].strip(), "question 2")
        self.assertTrue(calls[2]["stdin"].strip().startswith("question 0 ---"))


class TestWarnSharing(TempBench):
    def _config(self, text):
        path = os.path.join(self.tmp, "config.toml")
        open(path, "w").write(text)
        return path

    def test_sharing_on_is_silent(self):
        path = self._config('[ai]\nprovider = "x"\nallow_data_sharing = true\n')
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            bench.warn_sharing(path)
        self.assertEqual(out.getvalue(), "")

    def test_sharing_off_warns(self):
        path = self._config("[ai]\nprovider = \"x\"\n")
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            bench.warn_sharing(path)
        self.assertIn("allow_data_sharing", out.getvalue())

    def test_commented_out_true_does_not_count(self):
        path = self._config('[ai]\n# allow_data_sharing = true\n'
                            'allow_data_sharing = false\n')
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            bench.warn_sharing(path)
        self.assertIn("allow_data_sharing", out.getvalue())

    def test_missing_file_is_silent(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            bench.warn_sharing(os.path.join(self.tmp, "nope.toml"))
        self.assertEqual(out.getvalue(), "")


class TestRunGate(TempBench):
    def args(self, **kw):
        a = type("Args", (), {})()
        a.session = kw.get("session", True)
        a.candidates = kw.get("candidates", 1)
        a.suite, a.workers, a.timeout = "v1", 1, 600
        a.limit_per_db = 0
        return a

    def test_session_with_candidates_is_refused(self):
        with self.assertRaises(SystemExit):
            bench.cmd_run(self.args(candidates=3))

    def test_arm_names_ending_in_session_are_reserved(self):
        bench.ARM = "hotfix-session"
        with self.assertRaises(SystemExit):
            bench.cmd_run(self.args())
        bench.ARM = "default"

    def test_slash_question_is_refused_before_any_spawn(self):
        self.install_stub()
        os.environ["SAYA_AI_API_KEY"] = "test-key"
        os.makedirs(f"{bench.CORPUS}/spider_data", exist_ok=True)
        json.dump([{"db_id": "dbtest", "question": "/clear why", "query": "SELECT 1"}],
                  open(f"{bench.CORPUS}/spider_data/dev.json", "w"))
        with self.assertRaises(SystemExit):
            bench.cmd_run(self.args(session=True))
        self.assertEqual(self.calls(), [])
        # The same corpus runs fine without --session.
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            bench.cmd_run(self.args(session=False))
        self.assertEqual(len(self.calls()), 1)

    def test_limit_per_db_applies_before_the_question_gate(self):
        self.install_stub()
        os.environ["SAYA_AI_API_KEY"] = "test-key"
        os.makedirs(f"{bench.CORPUS}/spider_data", exist_ok=True)
        json.dump([{"db_id": "dbtest", "question": "fine", "query": "SELECT 1"},
                   {"db_id": "dbtest", "question": "/clear", "query": "SELECT 1"}],
                  open(f"{bench.CORPUS}/spider_data/dev.json", "w"))
        a = self.args()
        a.limit_per_db = 1
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            bench.cmd_run(a)                 # the /clear question is capped out
        self.assertEqual(len(self.calls()), 1)

    def test_default_summary_line_is_unchanged(self):
        self.install_stub()
        os.environ["SAYA_AI_API_KEY"] = "test-key"
        os.makedirs(f"{bench.CORPUS}/spider_data", exist_ok=True)
        json.dump([{"db_id": "dbtest", "question": "q", "query": "SELECT 1"}],
                  open(f"{bench.CORPUS}/spider_data/dev.json", "w"))
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            bench.cmd_run(self.args(session=False))
        self.assertIn("1 database units, 1 questions, 0 already done",
                      out.getvalue())
        self.assertNotIn("session", out.getvalue())


if __name__ == "__main__":
    unittest.main(verbosity=2)