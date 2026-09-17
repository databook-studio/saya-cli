"""S0 probe: saya as a Harbor installed agent, to record the honest floor.

NOT YET RUNNABLE. Two prerequisites are unbuilt:
  * `--turn-file` (design S1) — until it exists the instruction is piped and
    its markdown layout folds, which the probe stamps into trial metadata so
    a floor can never be quoted as if the input arrived intact;
  * a saya binary matching the container's architecture. Containers on Apple
    Silicon are aarch64 and CI publishes linux-x86_64 only, so one must be
    built in-container (see the aarch64 item in the internal TODO).

Verified against Harbor 0.23.0: `BaseAgent` requires name/version/setup/run,
`BaseInstalledAgent` adds install, and the oracle scores 1.0 on a real
Terminal-Bench 2.1 task locally under Docker with no auth.

Throwaway by design. It exists to answer three questions before any real
adapter is written:

  1. Does the contract reading hold against the installed Harbor?
  2. Does a saya episode complete inside a task container at all?
  3. What does saya score with no host-commands lane — the floor?

The floor is expected to be near zero: Terminal-Bench 2.1 tasks assume
package installation and network, and a lane-less saya has neither. A floor
is a diagnostic, not a result, and it is labelled as such wherever it is
quoted.

Not for reuse: it uploads a locally built binary and writes config into the
container. The real adapter (S3) pins a release binary by checksum.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import override

from harbor.agents.base import BaseAgent
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext

# Where the probe stages saya and its state inside the container. Everything
# lives under /logs/agent so Harbor syncs it back per trial — the artifact a
# reviewer reads is the same one the episode wrote.
_LOGS = "/logs/agent"
_BIN = f"{_LOGS}/saya"
_CONFIG_HOME = f"{_LOGS}/saya-config"
_INSTRUCTION = f"{_LOGS}/instruction.md"


class SayaProbeAgent(BaseAgent):
    """Drives saya's session surface headless for one episode."""

    @staticmethod
    @override
    def name() -> str:
        return "saya-probe"

    @override
    def version(self) -> str:
        return os.environ.get("SAYA_PROBE_VERSION", "0.4.1-probe")

    @override
    async def setup(self, environment: BaseEnvironment) -> None:
        """Upload the binary and generate config.

        The generated config is the `bench/spider/profiles.py` discipline:
        the container user's own configuration is never read or written, so
        an episode cannot inherit or corrupt developer settings.
        """
        binary = Path(
            os.environ.get("SAYA_PROBE_BINARY", "/tmp/saya-linux-out/saya-linux-arm64")
        )
        if not binary.is_file():
            raise FileNotFoundError(
                f"no saya binary at {binary} — build one for the container's "
                "architecture first; the probe does not build in-container"
            )
        await environment.exec(f"mkdir -p {_CONFIG_HOME}")
        await environment.upload_file(binary, _BIN)
        await environment.exec(f"chmod +x {_BIN}")

        # Minimal config: no connections, no telemetry. The probe measures the
        # coding surface, not the database one.
        await environment.exec(
            f"printf '%s\\n' '[run]' 'query_timeout_seconds = 60' > {_CONFIG_HOME}/config.toml"
        )

    @override
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        """One episode: the task instruction, verbatim, as a single turn.

        The instruction is written to a file rather than piped, because a TB
        instruction is multi-line markdown with code blocks and the piped
        REPL reads line by line — `bench/spider/bench.py` already records
        that cost: "the words survive, the layout does not". Losing layout
        on the exact input being graded would make the floor meaningless.

        `--turn-file` does not exist yet (it is S1). Until it does, the
        probe pipes and accepts the folding, and says so in its log so the
        floor is never read as if the input arrived intact.
        """
        await environment.exec(
            f"cat > {_INSTRUCTION} <<'SAYA_EOF'\n{instruction}\nSAYA_EOF"
        )

        # Piped stdin, read-only approvals: the lane does not exist yet, so
        # nothing write-shaped is reachable. This IS the floor.
        result = await environment.exec(
            f"cd /app && {_BIN} --approval-mode read-only "
            f"< {_INSTRUCTION} > {_LOGS}/saya.ndjson 2> {_LOGS}/saya.stderr; "
            f"echo exit=$? >> {_LOGS}/saya.stderr",
            env={"SAYA_CONFIG_HOME": _CONFIG_HOME, "HOME": _LOGS},
            timeout_sec=900,
        )
        context.metadata = {
            **(context.metadata or {}),
            "saya_probe_exit": getattr(result, "exit_code", None),
            "instruction_delivery": "piped-stdin (layout folded; --turn-file is S1)",
            "lane": "absent (floor measurement)",
        }
