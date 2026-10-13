"""Tiny private worktree fixture for inspector preflight tests."""

from contextlib import contextmanager
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

REPOSITORY = Path(__file__).resolve().parents[1]
EXAMPLES = ("connections.toml", "connections.docker.toml")


@contextmanager
def isolated_inspector_workspace(inspector: Path):
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        script = root / "scripts" / "inspect-crate-packages.sh"
        script.parent.mkdir(parents=True)
        shutil.copyfile(inspector, script)
        for name in EXAMPLES:
            for destination, source in (
                (root / "examples" / name, REPOSITORY / "examples" / name),
                (
                    root / "crates" / "saya-config" / "examples" / f"{name.removesuffix('.toml')}.example.toml",
                    REPOSITORY
                    / "crates"
                    / "saya-config"
                    / "examples"
                    / f"{name.removesuffix('.toml')}.example.toml",
                ),
            ):
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, destination)
        global_config = root / "isolated.gitconfig"
        global_config.write_text("")
        git_env = {
            **os.environ,
            "GIT_CONFIG_GLOBAL": str(global_config),
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_TERMINAL_PROMPT": "0",
        }
        hooks = root / "empty-hooks"
        hooks.mkdir()
        subprocess.run(
            ["git", "init", "--quiet"], cwd=root, env=git_env, check=True
        )
        subprocess.run(
            [
                "git",
                "add",
                "--",
                "scripts/inspect-crate-packages.sh",
                "examples/connections.toml",
                "examples/connections.docker.toml",
                "crates/saya-config/examples/connections.example.toml",
                "crates/saya-config/examples/connections.docker.example.toml",
            ],
            cwd=root,
            env=git_env,
            check=True,
        )
        subprocess.run(
            [
                "git",
                "-c",
                "commit.gpgsign=false",
                "-c",
                f"core.hooksPath={hooks.as_posix()}",
                "-c",
                "user.name=Inspector fixture",
                "-c",
                "user.email=inspector-fixture@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "seed inspector fixture",
            ],
            cwd=root,
            env=git_env,
            check=True,
        )
        yield root, script
