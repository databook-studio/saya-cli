# CI and release candidates

Pull requests and pushes run `.github/workflows/ci.yml` across Linux, macOS,
and Windows. The verification job runs formatting, the workspace test suite,
and strict Clippy. Live PostgreSQL/MySQL contract jobs run on every push and
pull request with ephemeral services; live Snowflake validation remains opt-in
and outside CI.

Every workspace crate inherits an MSRV of Rust 1.88, and the edition 2024
workspace uses Cargo resolver 3. A separate serialized Ubuntu job installs
Rust 1.88.0 exactly and runs `cargo check --workspace --locked`. This catches
dependency or source MSRV drift without duplicating the stable test matrix.

The workspace pins `duckdb` and its bundled `libduckdb-sys` implementation to
`1.10504.0`. This release vendors fmt without the obsolete MSVC
`stdext::checked_array_iterator` branch. Its published
[`libduckdb-sys` build script](https://docs.rs/crate/libduckdb-sys/1.10504.0/source/build_bundled_cc.rs)
enables `/EHsc` behind an MSVC target gate, so CI does not override dependency
C++ flags.

Builds compile in parallel. The earlier `CARGO_BUILD_JOBS=1` throttle was
removed — it serialized the ~1,800-file bundled DuckDB C++ compile and was the
dominant cost. Incremental compilation and test-profile debug info are disabled
to keep the cached `target/` small, and `Swatinem/rust-cache` caches the cargo
registry and `target/` (including the compiled DuckDB objects) between runs. The
cache is written only from `main`, so pull requests restore it without evicting
it. `ci.yml` runs on pull requests and pushes to `main`.

Releases use `.github/workflows/release-candidate.yml`, triggered by pushing a
`vX.Y.Z` tag (or manually with `publish: false` to validate without releasing).
Its build matrix covers Linux x86_64, macOS arm64, macOS x86_64 (cross-compiled
on the Apple Silicon runner, since GitHub's Intel runners are scarce), and
Windows x86_64. Each build compiles `saya` in release mode, smoke-tests
`saya --version` and `--non-interactive config doctor` on native targets, and
uploads a tar (Unix) or zip (Windows) archive with a SHA-256 sidecar. Tests and
Clippy are not re-run there — the branch ruleset gates `main` on them before a
tag is cut.

On a real tag, later jobs verify and aggregate `SHA256SUMS`, create the GitHub
Release, publish the workspace to crates.io, and bump the Homebrew tap formula.
See [`RELEASING.md`](../RELEASING.md) for the full sequence and the
`CARGO_REGISTRY_TOKEN` / `HOMEBREW_TAP_TOKEN` secrets those jobs use; each no-ops
when its secret is unset.

Do not treat a green build as a signed release. Signing requires external
credentials and a release plan; no signing step or fake signature is included.

Local parity is available with `scripts/package.sh`, which writes the archive
and `.sha256` file under `dist/` unless `SAYA_PACKAGE_DIR` is set.
Run `scripts/check-release-workflow.sh` to validate release and CI action
pins, MSRV inheritance and its exact CI gate, resource limits, the matched
bundled DuckDB pin, workflow YAML, publish permissions/gate, and the Windows
UTF-8/LF checksum sidecar contract.

## Troubleshooting: every job fails at "Set up job"

If all CI jobs fail within a few seconds with no steps executed, the cause is
GitHub Actions billing, not the code or workflow. A failed payment or an
exhausted Actions spending limit blocks the runners before any step runs; the
check annotation reads: "The job was not started because recent account
payments have failed or your spending limit needs to be increased." Resolve it
in the organization's **Settings → Billing and plans** (fix the payment method
or raise the Actions spending limit) and confirm **Settings → Actions** is
enabled. No workflow change is required; re-run the jobs once billing is
restored.
