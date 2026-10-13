#!/usr/bin/env python3
"""Behavior tests for the package-only workspace inspection entry.

The script under test must package EVERY workspace crate (never skipping an
"already published" one), scan each payload, and emit a JSON receipt — without
ever invoking `cargo publish` or contacting crates.io. Real cargo is not used:
every test runs against stub `cargo`, stub scanner interpreter, and a stub
`curl` that fails loudly if the inspection ever tries to reach the network.
"""

import hashlib
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from inspect_crate_packages_test_fixture import isolated_inspector_workspace

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "inspect-crate-packages.sh"

CRATES = [
    "saya-types",
    "saya-config",
    "saya-store",
    "saya-agent",
    "saya-connectors",
    "saya-harness",
    "saya-cli",
]
VERSION = "0.5.0"
STUB_CARGO_VERSION = "cargo 9.9.9 (inspect-test-stub)"

CARGO_STUB = """#!/usr/bin/env bash
set -euo pipefail
printf 'cargo %s\\n' "$*" >>"$INSPECT_TEST_LOG"
case "$1" in
  --version)
    echo "$STUB_CARGO_VERSION"
    ;;
  metadata)
    cat "$INSPECT_METADATA_JSON"
    ;;
  package)
    crate=""
    prev=""
    for arg in "$@"; do
      if [ "$prev" = "-p" ]; then crate="$arg"; fi
      prev="$arg"
    done
    if [ -n "${FAIL_PACKAGE_CRATE:-}" ] && [ "$crate" = "$FAIL_PACKAGE_CRATE" ]; then
      echo "stub cargo: package failed for $crate" >&2
      exit 1
    fi
    if [ -n "${SKIP_PACKAGE_CRATE:-}" ] && [ "$crate" = "$SKIP_PACKAGE_CRATE" ]; then
      exit 0
    fi
    version="$(jq -er --arg n "$crate" '[.packages[] | select(.name == $n) | .version] | .[0]' "$INSPECT_METADATA_JSON")"
    out="$CARGO_TARGET_DIR/package/$crate-$version.crate"
    mkdir -p "$(dirname "$out")"
    printf 'stub crate payload for %s\\n' "$crate" >"$out"
    ;;
  *)
    echo "stub cargo: unexpected invocation: $*" >&2
    exit 1
    ;;
esac
"""

PYTHON_STUB = """#!/usr/bin/env bash
set -euo pipefail
printf 'python %s\\n' "$*" >>"$INSPECT_TEST_LOG"
for arg in "$@"; do
  if [ -n "${FAIL_SCAN_CRATE:-}" ] && [ "${arg#*$FAIL_SCAN_CRATE-}" != "$arg" ]; then
    echo "artifact content check failed: stub rejection of $arg" >&2
    exit 1
  fi
done
echo "artifact content check passed: $*"
"""

CURL_STUB = """#!/usr/bin/env bash
printf 'curl %s\\n' "$*" >>"$INSPECT_TEST_LOG"
echo "stub curl: the inspection must never touch the network" >&2
exit 1
"""


def metadata_json(names: list[str], version: str = VERSION) -> dict:
    """Canned `cargo metadata` output: every crate depends on all earlier ones,
    which is a valid dependency order for the canonical CRATES list."""
    packages = []
    for index, name in enumerate(names):
        packages.append(
            {
                "id": f"{name} {version} (path+file:///crates/{name})",
                "name": name,
                "version": version,
                "dependencies": [
                    {"name": earlier, "req": f"^{version}", "path": f"../crates/{earlier}"}
                    for earlier in names[:index]
                ],
            }
        )
    return {
        "workspace_members": [package["id"] for package in packages],
        "packages": packages,
    }


class InspectPackagesTests(unittest.TestCase):
    def run_inspect(
        self,
        tmp: Path,
        names: list[str],
        env_extra: dict[str, str] | None = None,
        script: Path = SCRIPT,
    ) -> tuple[subprocess.CompletedProcess[str], Path, Path]:
        bin_dir = tmp / "bin"
        bin_dir.mkdir()
        metadata = tmp / "metadata.json"
        metadata.write_text(json.dumps(metadata_json(names)))
        log = tmp / "invocations.log"
        receipt = tmp / "receipt.json"
        for name, body in (("stub-cargo", CARGO_STUB), ("stub-python", PYTHON_STUB), ("curl", CURL_STUB)):
            stub = bin_dir / name
            stub.write_text(body)
            stub.chmod(0o755)
        base = {k: v for k, v in os.environ.items() if not k.startswith(("FAIL_", "SKIP_", "INSPECT_"))}
        env = {
            **base,
            "PATH": f"{bin_dir}{os.pathsep}{base.get('PATH', '')}",
            "CARGO": str(bin_dir / "stub-cargo"),
            "PYTHON": str(bin_dir / "stub-python"),
            "STUB_CARGO_VERSION": STUB_CARGO_VERSION,
            "CARGO_TARGET_DIR": str(tmp / "target"),
            "INSPECT_TEST_LOG": str(log),
            "INSPECT_METADATA_JSON": str(metadata),
            "INSPECT_RECEIPT": str(receipt),
            **(env_extra or {}),
        }
        result = subprocess.run(
            ["bash", str(script)], env=env, text=True, capture_output=True, check=False
        )
        return result, log, receipt

    def test_drifted_root_example_fails_before_packaging(self) -> None:
        for name in ("connections.toml", "connections.docker.toml"):
            with self.subTest(name=name):
                with isolated_inspector_workspace(SCRIPT) as (root, script):
                    (root / "examples" / name).write_text("drifted copy\n")
                    result, log, receipt_path = self.run_inspect(root, CRATES, script=script)
                    self.assertNotEqual(result.returncode, 0)
                    receipt = json.loads(receipt_path.read_text())
                    self.assertFalse(receipt["all_pass"])
                    self.assertEqual(receipt["failure"]["crate"], "saya-config")
                    self.assertIn(f"examples/{name}", receipt["failure"]["reason"])
                    self.assertNotIn("cargo metadata", log.read_text())
                    self.assertNotIn("cargo package", log.read_text())

    def receipt_from_stdout(self, result: subprocess.CompletedProcess[str]) -> dict:
        last = result.stdout.strip().splitlines()[-1]
        return json.loads(last)

    def test_all_crates_pass_and_receipt_is_complete(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_name:
            tmp = Path(tmp_name)
            result, log, receipt_path = self.run_inspect(tmp, CRATES)
            self.assertEqual(result.returncode, 0, result.stderr)
            receipt = json.loads(receipt_path.read_text())
            self.assertEqual(self.receipt_from_stdout(result), receipt)
            self.assertTrue(receipt["all_pass"])
            self.assertEqual(receipt["source"], self.head())
            self.assertIsInstance(receipt["dirty"], bool)
            self.assertEqual(receipt["cargo_version"], STUB_CARGO_VERSION)
            self.assertEqual([row["name"] for row in receipt["crates"]], CRATES)
            for row in receipt["crates"]:
                self.assertEqual(row["version"], VERSION)
                self.assertEqual(row["scan"], "pass")
                payload = tmp / "target" / "package" / f"{row['name']}-{row['version']}.crate"
                self.assertEqual(Path(row["package_path"]), payload)
                data = payload.read_bytes()
                self.assertEqual(row["bytes"], len(data))
                self.assertEqual(row["sha256"], hashlib.sha256(data).hexdigest())
            for line in log.read_text().splitlines():
                if line.startswith("cargo package"):
                    self.assertNotIn("--offline", line)

    def test_missing_payload_fails_without_all_pass(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_name:
            tmp = Path(tmp_name)
            result, _, receipt_path = self.run_inspect(tmp, CRATES, {"SKIP_PACKAGE_CRATE": "saya-store"})
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(json.loads(receipt_path.read_text())["all_pass"])
            self.assertNotIn('"all_pass":true', result.stdout)
            self.assertEqual(json.loads(receipt_path.read_text())["failure"]["crate"], "saya-store")

    def test_scanner_failure_fails_and_names_the_crate(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_name:
            tmp = Path(tmp_name)
            result, _, receipt_path = self.run_inspect(tmp, CRATES, {"FAIL_SCAN_CRATE": "saya-store"})
            self.assertNotEqual(result.returncode, 0)
            receipt = json.loads(receipt_path.read_text())
            self.assertFalse(receipt["all_pass"])
            self.assertNotIn('"all_pass":true', result.stdout)
            self.assertEqual(receipt["failure"]["crate"], "saya-store")

    def test_no_publish_no_network_no_skip(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_name:
            tmp = Path(tmp_name)
            result, log, receipt_path = self.run_inspect(tmp, CRATES)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(json.loads(receipt_path.read_text())["all_pass"])
            lines = log.read_text().splitlines()
            self.assertTrue(lines, "stub tools were never invoked")
            for line in lines:
                self.assertNotIn("publish", line)
                self.assertNotIn("curl", line)
            package_lines = [line for line in lines if line.startswith("cargo package")]
            self.assertEqual(len(package_lines), len(CRATES), "every crate must be packaged exactly once")
            for crate, line in zip(CRATES, package_lines):
                self.assertIn(f"-p {crate}", line)

    def test_crate_count_mismatch_fails_before_packaging(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_name:
            tmp = Path(tmp_name)
            result, log, receipt_path = self.run_inspect(tmp, CRATES + ["saya-extra"])
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(json.loads(receipt_path.read_text())["all_pass"])
            self.assertNotIn('"all_pass":true', result.stdout)
            self.assertFalse([line for line in log.read_text().splitlines() if line.startswith("cargo package")])

    def test_offline_flag_is_forwarded_to_cargo_package(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_name:
            tmp = Path(tmp_name)
            result, log, receipt_path = self.run_inspect(tmp, CRATES, {"INSPECT_OFFLINE": "1"})
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(json.loads(receipt_path.read_text())["all_pass"])
            package_lines = [line for line in log.read_text().splitlines() if line.startswith("cargo package")]
            self.assertEqual(len(package_lines), len(CRATES))
            for line in package_lines:
                self.assertIn("--offline", line)

    def head(self) -> str:
        return subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=True,
        ).stdout.strip()


if __name__ == "__main__":
    unittest.main()
