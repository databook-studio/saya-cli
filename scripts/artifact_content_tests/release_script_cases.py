from .test_support import *

class ReleaseScriptCases:
    def executable(self, path: Path, content: str) -> None:
        path.write_text(content)
        path.chmod(0o755)

    def test_release_build_env_preserves_flags_and_paths_with_spaces(self) -> None:
        helper = ROOT / "scripts" / "release-build-env.sh"
        command = (
            f'source "{helper}"; configure_saya_release_build_env "$WORKSPACE"; '
            'printf "%s\\0%s\\0%s" "$CARGO_ENCODED_RUSTFLAGS" "$CXXFLAGS" "$CC_SHELL_ESCAPED_FLAGS"'
        )
        env = os.environ | {
            "WORKSPACE": "/tmp/saya workspace",
            "CARGO_HOME": "/tmp/cargo home",
            "CARGO_TARGET_DIR": "/tmp/build target",
            "CARGO_ENCODED_RUSTFLAGS": "-C\x1fopt-level=3",
            "RUSTFLAGS": "-C opt-level=1",
            "CXXFLAGS": "-DSAYA_EXISTING=1",
        }
        result = subprocess.run(["bash", "-c", command], cwd=ROOT, env=env, capture_output=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        encoded, cxxflags, escaped = result.stdout.decode().split("\0")
        self.assertEqual(encoded.split("\x1f"), [
            "-C", "opt-level=3",
            "--remap-path-prefix=/tmp/saya workspace=/saya",
            "--remap-path-prefix=/tmp/cargo home=/cargo",
        ])
        parsed_cxx = shlex.split(cxxflags)
        self.assertEqual(parsed_cxx, [
            "-DSAYA_EXISTING=1",
            "-ffile-prefix-map=/tmp/build target=/saya-target",
            "-ffile-prefix-map=/tmp/saya workspace=/saya",
            "-ffile-prefix-map=/tmp/cargo home=/cargo",
        ])
        self.assertEqual(escaped, "1")
        legacy_env = dict(env)
        legacy_env.pop("CARGO_ENCODED_RUSTFLAGS")
        legacy_env.pop("RUSTFLAGS")
        legacy_env["RUSTFLAGS"] = "-C debuginfo=0"
        legacy = subprocess.run(["bash", "-c", command], cwd=ROOT, env=legacy_env, capture_output=True, check=False)
        self.assertEqual(legacy.returncode, 0, legacy.stderr.decode())
        legacy_encoded = legacy.stdout.decode().split("\0")[0].split("\x1f")
        self.assertEqual(legacy_encoded, [
            "-C", "debuginfo=0",
            "--remap-path-prefix=/tmp/saya workspace=/saya",
            "--remap-path-prefix=/tmp/cargo home=/cargo",
        ])

    def test_release_build_env_remaps_relative_and_default_target_dirs(self) -> None:
        helper = ROOT / "scripts" / "release-build-env.sh"
        command = (
            f'source "{helper}"; configure_saya_release_build_env "$WORKSPACE"; '
            'printf "%s\\0%s" "$CARGO_ENCODED_RUSTFLAGS" "$CXXFLAGS"'
        )
        for target_dir, expected in (("build target", "/tmp/saya workspace/build target"), (None, "/tmp/saya workspace/target")):
            env = os.environ | {"WORKSPACE": "/tmp/saya workspace", "CARGO_HOME": "/tmp/cargo home"}
            env.pop("CARGO_ENCODED_RUSTFLAGS", None)
            env.pop("RUSTFLAGS", None)
            env.pop("CXXFLAGS", None)
            if target_dir is None:
                env.pop("CARGO_TARGET_DIR", None)
            else:
                env["CARGO_TARGET_DIR"] = target_dir
            result = subprocess.run(["bash", "-c", command], cwd=ROOT, env=env, capture_output=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            rustflags, cxxflags = result.stdout.decode().split("\0")
            self.assertEqual(rustflags.split("\x1f"), [
                "--remap-path-prefix=/tmp/saya workspace=/saya",
                "--remap-path-prefix=/tmp/cargo home=/cargo",
            ])
            self.assertEqual(shlex.split(cxxflags), [
                f"-ffile-prefix-map={expected}=/saya-target",
                "-ffile-prefix-map=/tmp/saya workspace=/saya",
                "-ffile-prefix-map=/tmp/cargo home=/cargo",
            ])

    def test_release_build_env_preserves_multiline_and_encoded_argument_flags(self) -> None:
        helper = ROOT / "scripts" / "release-build-env.sh"
        command = (
            f'source "{helper}"; configure_saya_release_build_env "$WORKSPACE"; '
            'printf "%s" "$CARGO_ENCODED_RUSTFLAGS"'
        )
        base = os.environ | {
            "WORKSPACE": "/tmp/saya workspace",
            "CARGO_HOME": "/tmp/cargo home",
        }
        legacy_env = dict(base)
        legacy_env.pop("CARGO_ENCODED_RUSTFLAGS", None)
        legacy_env["RUSTFLAGS"] = "-C opt-level=3\n-C debuginfo=0"
        legacy = subprocess.run(["bash", "-c", command], cwd=ROOT, env=legacy_env, capture_output=True, check=False)
        self.assertEqual(legacy.returncode, 0, legacy.stderr.decode())
        self.assertEqual(legacy.stdout.decode().split("\x1f"), [
            "-C", "opt-level=3", "-C", "debuginfo=0",
            "--remap-path-prefix=/tmp/saya workspace=/saya",
            "--remap-path-prefix=/tmp/cargo home=/cargo",
        ])

        encoded_env = dict(base)
        encoded_env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f--cfg\x1ffeature=custom build\x1f"
        encoded_env["RUSTFLAGS"] = "-C opt-level=3\n-C debuginfo=0"
        encoded = subprocess.run(["bash", "-c", command], cwd=ROOT, env=encoded_env, capture_output=True, check=False)
        self.assertEqual(encoded.returncode, 0, encoded.stderr.decode())
        self.assertEqual(encoded.stdout.decode().split("\x1f"), [
            "", "--cfg", "feature=custom build", "",
            "--remap-path-prefix=/tmp/saya workspace=/saya",
            "--remap-path-prefix=/tmp/cargo home=/cargo",
        ])

        empty_encoded_env = dict(encoded_env)
        empty_encoded_env["CARGO_ENCODED_RUSTFLAGS"] = ""
        empty_encoded = subprocess.run(["bash", "-c", command], cwd=ROOT, env=empty_encoded_env, capture_output=True, check=False)
        self.assertEqual(empty_encoded.returncode, 0, empty_encoded.stderr.decode())
        self.assertEqual(empty_encoded.stdout.decode().split("\x1f"), [
            "--remap-path-prefix=/tmp/saya workspace=/saya",
            "--remap-path-prefix=/tmp/cargo home=/cargo",
        ])

    def test_package_script_uses_custom_target_dir_with_spaces(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bin_dir = root / "stub bin"
            bin_dir.mkdir()
            target = root / "custom target dir"
            output = root / "package output"
            stub_binary = root / "stub saya"
            stub_binary.write_text(
                "#!/bin/sh\n"
                'if [ "$1" = "--version" ]; then echo STUB_BUILD_OUTPUT; exit 0; fi\n'
                'if [ "$1" = "--non-interactive" ]; then exit 3; fi\n'
                "exit 0\n"
            )
            stub_binary.chmod(0o755)
            self.executable(bin_dir / "rustc", "#!/bin/sh\necho 'host: x86_64-unknown-linux-gnu'\n")
            self.executable(
                bin_dir / "cargo",
                "#!/bin/sh\nset -eu\n"
                'test "$1" = build\n'
                'mkdir -p "$CARGO_TARGET_DIR/release"\n'
                'cp "$SAYA_STUB_BINARY" "$CARGO_TARGET_DIR/release/saya"\n'
                'chmod +x "$CARGO_TARGET_DIR/release/saya"\n',
            )
            env = os.environ | {
                "PATH": f"{bin_dir}:{os.environ['PATH']}",
                "CARGO_TARGET_DIR": str(target),
                "SAYA_PACKAGE_DIR": str(output),
                "SAYA_STUB_BINARY": str(stub_binary),
            }
            result = subprocess.run(["bash", "scripts/package.sh"], cwd=ROOT, env=env, text=True, capture_output=True, check=False)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((target / "release" / "saya").exists())
            archive_path = next(output.glob("*.tar.gz"))
            prefix = archive_path.name.removesuffix(".tar.gz")
            with tarfile.open(archive_path, "r:gz") as archive:
                self.assertEqual(archive.extractfile(f"{prefix}/saya").read(), stub_binary.read_bytes())

    def test_publish_script_uses_custom_target_dir_with_spaces(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bin_dir = root / "stub bin"
            fixtures = root / "crate fixtures"
            target = root / "custom target dir"
            bin_dir.mkdir()
            fixtures.mkdir()
            metadata = subprocess.run(
                ["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"],
                cwd=ROOT, text=True, capture_output=True, check=True,
            ).stdout
            packages = ["saya-types", "saya-config", "saya-store", "saya-agent", "saya-connectors", "saya-harness", "saya-cli"]
            for crate in packages:
                fixture = fixtures / f"{crate}-0.4.2.crate"
                valid_crate(fixture, {"src/lib.rs": b"pub fn packaged() {}"})
            self.executable(
                bin_dir / "cargo",
                "#!/bin/sh\nset -eu\n"
                'case "$1" in\n'
                'metadata) cat "$SAYA_METADATA_FIXTURE" ;;\n'
                'package) shift; crate=""; while [ "$#" -gt 0 ]; do if [ "$1" = -p ]; then crate="$2"; shift 2; else shift; fi; done; mkdir -p "$CARGO_TARGET_DIR/package"; cp "$SAYA_CRATE_FIXTURES/$crate-0.4.2.crate" "$CARGO_TARGET_DIR/package/$crate-0.4.2.crate" ;;\n'
                'publish) exit 0 ;;\n'
                '*) exit 2 ;;\n'
                'esac\n',
            )
            self.executable(bin_dir / "curl", "#!/bin/sh\nprintf 404\n")
            metadata_path = root / "metadata.json"
            metadata_path.write_text(metadata)
            env = os.environ | {
                "PATH": f"{bin_dir}:{os.environ['PATH']}",
                "CARGO_TARGET_DIR": str(target),
                "SAYA_METADATA_FIXTURE": str(metadata_path),
                "SAYA_CRATE_FIXTURES": str(fixtures),
                "DRY_RUN": "1",
            }
            result = subprocess.run(["bash", "scripts/publish-crates.sh"], cwd=ROOT, env=env, text=True, capture_output=True, check=False)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(len(list((target / "package").glob("*.crate"))), len(packages))
            self.assertEqual(result.stdout.count(f"artifact content check passed: crate {target}/package/"), len(packages))
