#!/usr/bin/env python3
"""Behavior tests for the release artifact content gate."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from artifact_content_tests.archive_content_cases import ArchiveContentCases
from artifact_content_tests.archive_structure_cases import ArchiveStructureCases
from artifact_content_tests.release_script_cases import ReleaseScriptCases
from artifact_content_tests.test_support import *


class ArtifactContentTests(ArchiveStructureCases, ArchiveContentCases, unittest.TestCase):
    def check(self, kind: str, path: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(GATE), kind, str(path)],
            text=True,
            capture_output=True,
            check=False,
        )

    def assert_rejected(self, kind: str, path: Path) -> None:
        result = self.check(kind, path)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("artifact content check failed:", result.stderr)


class ReleaseScriptIntegrationTests(ReleaseScriptCases, unittest.TestCase):
    pass


if __name__ == "__main__":
    unittest.main()
