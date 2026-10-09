"""Tests for source enumeration over git views."""

import os
import unittest

import fixtures
from file_limits.counting import count_physical_lines
from file_limits.policy import load_policy
from file_limits.views import enumerate_view, is_eligible


class EnumerateViewTests(unittest.TestCase):
    def setUp(self):
        self.repo = fixtures.TempRepo()
        self.addCleanup(self.repo.destroy)

    def policy(self):
        return load_policy(self.repo.path / ".config" / "file-limits.json")

    def counts(self, view):
        sources, errors = enumerate_view(self.repo.path, view, self.policy())
        self.assertEqual(errors, [])
        return {path: count_physical_lines(data) for path, data in sources}

    def test_index_counts_staged_bytes_without_a_commit(self):
        self.repo.write_lines("src/lib.rs", 251)
        self.repo.git("add", "--", "src/lib.rs")
        self.assertEqual(self.counts("index"), {"src/lib.rs": 251})

    def test_worktree_reads_disk_and_index_ignores_untracked(self):
        self.repo.write_lines("src/lib.rs", 10)
        self.repo.commit_all("base")
        self.repo.write_lines("untracked.rs", 251)
        self.assertEqual(self.counts("worktree")["untracked.rs"], 251)
        self.assertNotIn("untracked.rs", self.counts("index"))

    def test_ignored_untracked_file_is_not_enumerated(self):
        self.repo.write(".gitignore", b"ign.py\n")
        self.repo.write_lines("ign.py", 251)
        self.repo.commit_all("base")
        self.assertEqual(self.counts("worktree"), {})

    def test_rev_head_minus_one_judges_the_older_tree(self):
        self.repo.write_lines("src/a.rs", 10)
        self.repo.commit_all("old")
        self.repo.write_lines("src/a.rs", 251)
        self.repo.commit_all("new")
        self.assertEqual(self.counts("rev:HEAD~1"), {"src/a.rs": 10})
        self.assertEqual(self.counts("rev:HEAD"), {"src/a.rs": 251})

    def test_ineligible_paths_are_absent_from_every_view(self):
        self.repo.write_lines("docs/x.md", 251)
        self.repo.write_lines("target/debug/gen.rs", 251)
        self.repo.commit_all("base")
        for view in ("index", "worktree", "rev:HEAD"):
            self.assertEqual(self.counts(view), {})

    def test_tracked_symlink_is_an_error_in_index_and_worktree(self):
        self.repo.write("real.txt", b"content\n")
        os.symlink("real.txt", self.repo.path / "x.rs")
        self.repo.git("add", "-A")
        for view in ("index", "worktree"):
            sources, errors = enumerate_view(self.repo.path, view, self.policy())
            self.assertEqual(sources, [])
            self.assertEqual(len(errors), 1, errors)
            self.assertIn("x.rs", errors[0])

    def test_unreadable_worktree_file_is_an_error(self):
        self.repo.write_lines("src/lib.rs", 10)
        self.repo.commit_all("base")
        target = self.repo.path / "src" / "lib.rs"
        target.chmod(0o000)
        try:
            sources, errors = enumerate_view(self.repo.path, "worktree", self.policy())
        finally:
            target.chmod(0o644)
        self.assertEqual(sources, [])
        self.assertEqual(len(errors), 1, errors)
        self.assertIn("src/lib.rs", errors[0])


class IsEligibleTests(unittest.TestCase):
    def setUp(self):
        repo = fixtures.TempRepo()
        self.addCleanup(repo.destroy)
        self.policy = load_policy(repo.path / ".config" / "file-limits.json")

    def test_policy_extensions_are_eligible(self):
        for path in ("src/lib.rs", "tool.py", "run.sh", "build.rs", "examples/x.rs"):
            self.assertTrue(is_eligible(path, self.policy), path)

    def test_other_extensions_are_not_eligible(self):
        for path in ("docs/x.md", "data/x.json", "Cargo.toml", "Makefile"):
            self.assertFalse(is_eligible(path, self.policy), path)

    def test_excluded_dir_names_are_not_eligible(self):
        self.assertFalse(is_eligible("target/debug/lib.rs", self.policy))
        self.assertFalse(is_eligible("a/target/b.rs", self.policy))
        self.assertFalse(is_eligible("src/node_modules/thing.py", self.policy))

    def test_directory_name_prefixes_do_not_exclude(self):
        self.assertTrue(is_eligible("targets/lib.rs", self.policy))


if __name__ == "__main__":
    unittest.main()