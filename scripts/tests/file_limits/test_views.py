"""Tests for source enumeration over git views."""

import os
import shutil
import subprocess
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

    def assert_compiled_module_is_counted(self, source_path, declaration):
        module = "\n".join(f"pub fn item_{line}() {{}}" for line in range(252)) + "\n"
        self.repo.write("src/lib.rs", f"{declaration}\n".encode())
        self.repo.write(source_path, module.encode())
        rustc = shutil.which("rustc")
        if rustc:
            proc = subprocess.run(
                [rustc, "--crate-type", "lib", "--emit", "metadata", "-o",
                 str(self.repo.path / "lib.rmeta"), str(self.repo.path / "src/lib.rs")],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr.decode("utf-8", "replace"))
        self.repo.commit_all("compiled source fixture")
        for view in ("worktree", "index", "rev:HEAD"):
            self.assertEqual(self.counts(view).get(source_path), 252, view)

    def test_uppercase_rust_suffix_is_counted_in_every_git_view(self):
        self.assert_compiled_module_is_counted(
            "src/extra.RS", '#[path = "extra.RS"] pub mod extra;'
        )

    def test_tracked_src_target_module_is_counted_in_every_git_view(self):
        self.assert_compiled_module_is_counted(
            "src/target/mod.rs", "pub mod target;"
        )

    def test_nested_target_and_node_modules_paths_are_counted(self):
        module = "\n".join(f"def item_{line}(): pass" for line in range(252)) + "\n"
        paths = (
            "scripts/target/fixture.py",
            "src/node_modules/fixture.py",
            "tools/target/fixture.py",
        )
        for path in paths:
            self.repo.write(path, module.encode())
        self.repo.commit_all("source-owned directories")
        for view in ("worktree", "index", "rev:HEAD"):
            counts = self.counts(view)
            for path in paths:
                self.assertEqual(counts.get(path), 252, (view, path))

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
        self.repo.write_lines("node_modules/package/index.py", 251)
        self.repo.write_lines("crates/saya/target/debug/gen.rs", 251)
        self.repo.write_lines("crates/saya/node_modules/package/index.py", 251)
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

    def test_output_locations_are_excluded_and_other_nested_paths_count(self):
        self.assertFalse(is_eligible(".git/config.py", self.policy))
        self.assertFalse(is_eligible("target/debug/lib.rs", self.policy))
        self.assertFalse(is_eligible("node_modules/package/index.py", self.policy))
        self.assertTrue(is_eligible("a/target/b.rs", self.policy))
        self.assertFalse(is_eligible("crates/saya/target/b.rs", self.policy))
        self.assertFalse(is_eligible("crates/saya/node_modules/index.py", self.policy))
        self.assertTrue(is_eligible("src/node_modules/thing.py", self.policy))

    def test_directory_name_prefixes_do_not_exclude(self):
        self.assertTrue(is_eligible("targets/lib.rs", self.policy))


if __name__ == "__main__":
    unittest.main()
