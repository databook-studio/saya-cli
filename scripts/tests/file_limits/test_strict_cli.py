"""End-to-end strict-mode CLI tests."""

import json
import os
import shutil
import unittest

import fixtures

OVER = "251 lines, limit 250, excess 1"


class StrictCliTests(unittest.TestCase):
    def setUp(self):
        self.repo = fixtures.TempRepo()
        self.addCleanup(self.repo.destroy)

    def run_cli(self, *args):
        return fixtures.run_checker(self.repo.path, *args)

    def stage(self, relpath, count):
        self.repo.write_lines(relpath, count)
        self.repo.git("add", "--", relpath)

    def test_clean_repo_passes_with_summary_line(self):
        self.repo.write_lines("src/lib.rs", 10)
        self.repo.write_lines("scripts/tool.sh", 5)
        self.repo.commit_all("base")
        proc = self.run_cli("--view", "index")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("2 files checked, 0 over 250", proc.stdout)

    def test_index_rejects_oversized_copy_despite_legacy_allowance(self):
        self.repo.write_lines("src/original.rs", 250)
        self.repo.write(
            fixtures.BASELINE_RELPATH,
            fixtures.baseline_bytes(250, [("src/copied.rs", 251)]),
        )
        self.repo.commit_all("base")
        self.stage("src/copied.rs", 251)
        proc = self.run_cli("--view", "index")
        self.assertEqual(proc.returncode, 1, proc.stderr)
        self.assertIn("src/copied.rs: 251 lines, limit 250, excess 1", proc.stdout)

        migration = self.run_cli("--mode", "ratchet", "--base", "HEAD")
        self.assertEqual(migration.returncode, 2, migration.stderr)
        self.assertIn("unrecognized arguments", migration.stderr)

    def test_index_rejects_staged_oversized_copy_and_rename(self):
        for path in ("src/copy_source.rs", "src/rename_source.rs"):
            self.repo.write_lines(path, 250)
        self.repo.commit_all("base")

        self.repo.write_lines("src/copy_source.rs", 251)
        source = self.repo.path / "src/copy_source.rs"
        copied = self.repo.path / "src/copied.rs"
        shutil.copyfile(source, copied)
        self.assertEqual(copied.read_bytes(), source.read_bytes())
        self.repo.git("add", "--", "src/copied.rs")

        self.repo.write_lines("src/rename_source.rs", 251)
        self.repo.git("add", "--", "src/rename_source.rs")
        self.repo.git("mv", "src/rename_source.rs", "src/renamed.rs")

        proc = self.run_cli("--view", "index")
        self.assertEqual(proc.returncode, 1, proc.stderr)
        self.assertIn(f"src/copied.rs: {OVER}", proc.stdout)
        self.assertIn(f"src/renamed.rs: {OVER}", proc.stdout)
        self.assertNotIn("src/rename_source.rs:", proc.stdout)

    def test_over_limit_file_fails_with_details(self):
        self.repo.write_lines("src/lib.rs", 251)
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 1, proc.stderr)
        self.assertIn("src/lib.rs: 251 lines, limit 250, excess 1", proc.stdout)

    def test_violations_are_sorted_by_count_descending(self):
        self.repo.write_lines("src/big.rs", 300)
        self.repo.write_lines("src/small.rs", 260)
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 1, proc.stderr)
        self.assertLess(proc.stdout.index("src/big.rs"), proc.stdout.index("src/small.rs"))

    def test_json_report_has_the_documented_keys(self):
        self.repo.write_lines("src/lib.rs", 252)
        self.repo.write_lines("src/ok.rs", 10)
        proc = self.run_cli("--json")
        self.assertEqual(proc.returncode, 1, proc.stderr)
        document = json.loads(proc.stdout)
        self.assertEqual(
            set(document),
            {"mode", "view", "limit", "files_checked", "violations", "errors"},
        )
        self.assertEqual(document["mode"], "strict")
        self.assertEqual(document["view"], "worktree")
        self.assertEqual(document["limit"], 250)
        self.assertEqual(document["files_checked"], 2)
        self.assertEqual(document["errors"], [])
        self.assertEqual(len(document["violations"]), 1)
        entry = document["violations"][0]
        self.assertEqual(set(entry), {"path", "lines", "limit", "excess"})
        self.assertEqual(entry["path"], "src/lib.rs")
        self.assertEqual(entry["lines"], 252)
        self.assertEqual(entry["excess"], entry["lines"] - entry["limit"])

    def test_251_line_file_under_tests_fails(self):
        self.repo.write_lines("tests/helper.rs", 251)
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 1, proc.stderr)
        self.assertIn("tests/helper.rs", proc.stdout)

    def test_build_rs_and_examples_251_fail(self):
        self.repo.write_lines("build.rs", 251)
        self.repo.write_lines("examples/x.rs", 251)
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 1, proc.stderr)
        self.assertIn("build.rs", proc.stdout)
        self.assertIn("examples/x.rs", proc.stdout)

    def test_python_shell_and_ruby_251_fail(self):
        self.repo.write_lines("scripts/tool.py", 251)
        self.repo.write_lines("scripts/tool.sh", 251)
        self.repo.write_lines("scripts/check.rb", 251)
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 1, proc.stderr)
        self.assertIn("scripts/tool.py", proc.stdout)
        self.assertIn("scripts/tool.sh", proc.stdout)
        self.assertIn("scripts/check.rb", proc.stdout)

    def test_other_extensions_and_excluded_dirs_are_ignored(self):
        self.repo.write_lines("docs/x.md", 251)
        self.repo.write_lines("data/x.json", 251)
        self.repo.write_lines("Cargo.toml", 251)
        self.repo.write_lines("target/debug/gen.rs", 251)
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("0 files checked, 0 over 250", proc.stdout)

    def test_spaces_and_unicode_names_are_checked(self):
        self.repo.write_lines("src/my file.rs", 251)
        self.repo.write_lines("src/naïve éł.rs", 251)
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 1, proc.stderr)
        self.assertIn("src/my file.rs", proc.stdout)
        self.assertIn("src/naïve éł.rs", proc.stdout)

    def test_newline_in_name_works_in_index_json(self):
        self.repo.write_lines("src/we\nird.rs", 251)
        self.repo.git("add", "--", "src/we\nird.rs")
        proc = self.run_cli("--view", "index", "--json")
        self.assertEqual(proc.returncode, 1, proc.stderr)
        entry = json.loads(proc.stdout)["violations"][0]
        self.assertEqual(entry["path"], "src/we\nird.rs")

    def test_symlink_source_is_an_error(self):
        self.repo.write("real.txt", b"content\n")
        os.symlink("real.txt", self.repo.path / "x.rs")
        self.repo.git("add", "-A")
        for view in ("index", "worktree"):
            proc = self.run_cli("--view", view)
            self.assertEqual(proc.returncode, 3, proc.stderr)
            self.assertIn("x.rs", proc.stderr)

    def test_index_judges_staged_bytes_not_worktree(self):
        self.stage("src/lib.rs", 251)
        self.repo.write_lines("src/lib.rs", 10)
        self.assertEqual(self.run_cli("--view", "index").returncode, 1)
        self.assertEqual(self.run_cli("--view", "worktree").returncode, 0)

    def test_worktree_judges_disk_bytes_not_staged(self):
        self.stage("src/lib.rs", 10)
        self.repo.write_lines("src/lib.rs", 251)
        self.assertEqual(self.run_cli("--view", "index").returncode, 0)
        self.assertEqual(self.run_cli("--view", "worktree").returncode, 1)

    def test_untracked_file_reported_by_worktree_absent_from_index(self):
        self.repo.write_lines("src/lib.rs", 10)
        self.repo.commit_all("base")
        self.repo.write_lines("over.rs", 251)
        index_proc = self.run_cli("--view", "index")
        self.assertEqual(index_proc.returncode, 0, index_proc.stderr)
        self.assertNotIn("over.rs", index_proc.stdout)
        work_proc = self.run_cli("--view", "worktree")
        self.assertEqual(work_proc.returncode, 1, work_proc.stderr)
        self.assertIn(f"over.rs: {OVER}", work_proc.stdout)

    def test_rev_head_minus_one_judges_the_older_tree(self):
        self.repo.write_lines("src/a.rs", 10)
        self.repo.commit_all("old")
        self.repo.write_lines("src/a.rs", 251)
        self.repo.commit_all("new")
        self.assertEqual(self.run_cli("--view", "rev:HEAD~1").returncode, 0)
        self.assertEqual(self.run_cli("--view", "rev:HEAD").returncode, 1)

    def test_missing_config_is_an_error(self):
        (self.repo.path / ".config" / "file-limits.json").unlink()
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 3)
        self.assertIn("file-limits.json", proc.stderr)

    def test_invalid_json_config_is_an_error(self):
        self.repo.write(".config/file-limits.json", b"{not json")
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 3)
        self.assertIn("error", proc.stderr)

    def test_config_with_unknown_key_is_an_error(self):
        self.repo.write_policy(dict(fixtures.STANDARD_POLICY, bogus=1))
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 3)
        self.assertIn("bogus", proc.stderr)

    def test_config_with_missing_key_is_an_error(self):
        data = {k: v for k, v in fixtures.STANDARD_POLICY.items() if k != "review"}
        self.repo.write_policy(data)
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 3)
        self.assertIn("review", proc.stderr)

    def test_config_with_wrong_value_type_is_an_error(self):
        self.repo.write_policy(dict(fixtures.STANDARD_POLICY, limit="250"))
        proc = self.run_cli()
        self.assertEqual(proc.returncode, 3)
        self.assertIn("limit", proc.stderr)


if __name__ == "__main__":
    unittest.main()
