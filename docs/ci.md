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

Full-matrix and release-candidate builds set `CARGO_BUILD_JOBS=1` and disable
test-profile debug information. This serializes the largest native links and
keeps test binaries small enough for hosted runners while still compiling and
running the complete test suite.

Release candidates use `.github/workflows/release-candidate.yml`. Start it
manually from Actions. Each native runner:

1. runs `cargo fmt --check`, locked workspace tests, and strict locked Clippy;
2. builds `saya-cli` in release mode;
3. smoke-tests `saya --version` and `--non-interactive config doctor`;
4. uploads a native tar archive on Unix or zip archive on Windows plus a
   SHA-256 sidecar.

The local verification checklist additionally runs the workspace tests and
Clippy with `--offline` when the dependency cache is present.

The checksum job downloads every build artifact, verifies each sidecar, and
uploads one `SHA256SUMS` manifest. The `publish` input defaults to `false`.
Only an explicit `publish: true` dispatch can run the `gh release create` job,
which depends on both build completion and checksum verification.

Do not treat a green build as a signed release. Signing requires external
credentials and a release plan; no signing step or fake signature is included.
The workflow does not make crates.io or Homebrew releases.

Local parity is available with `scripts/package.sh`, which writes the archive
and `.sha256` file under `dist/` unless `SAYA_PACKAGE_DIR` is set.
Run `scripts/check-release-workflow.sh` to validate release and CI action
pins, MSRV inheritance and its exact CI gate, resource limits, the matched
bundled DuckDB pin, workflow YAML, publish permissions/gate, and the Windows
UTF-8/LF checksum sidecar contract.
