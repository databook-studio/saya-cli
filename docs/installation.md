# Installation

SAYA CLI ships prebuilt binaries for macOS (Apple Silicon + Intel), Linux
(x86_64), and Windows (x86_64) on every
[release](https://github.com/databook-studio/saya-cli/releases), and is
published on crates.io. Building from source requires Rust 1.88.0 or newer.

## Install a release build

```bash
# Homebrew (macOS / Linux)
brew install databook-studio/tap/saya

# Cargo, prebuilt binary, no compile (https://github.com/cargo-bins/cargo-binstall)
cargo binstall saya-cli

# Cargo, from source (compiles the bundled DuckDB — allow a few minutes)
cargo install saya-cli
```

All of these install a single `saya` binary onto your `PATH`.

## Build from a checkout

```bash
git clone https://github.com/databook-studio/saya-cli.git
cd saya-cli
cargo build --release --locked -p saya-cli
./target/release/saya --version
```

Install the binary into Cargo's user bin directory when desired:

```bash
cargo install --locked --path crates/saya-cli
saya --version
```

You can also install the latest source straight from git:

```bash
cargo install --locked --git https://github.com/databook-studio/saya-cli.git saya-cli
```

Prebuilt archives are produced by the release workflow on each version tag and
attached to the GitHub release alongside a `SHA256SUMS` manifest.

## First five minutes

```bash
saya config init
${EDITOR:-vi} .saya/config.toml
${EDITOR:-vi} .saya/connections.toml
export SAYA_ANALYTICS_PASSWORD='use-a-read-only-password'
saya config doctor
saya connection test analytics
saya --profile analytics --approval-mode read-only query --sql 'SELECT 1'
```

`config init` refuses to overwrite either file, uses restrictive Unix modes,
and makes a best-effort rollback after an ordinary second-file creation error;
it is not crash-atomic. The generated connection contains an environment
SecretRef, not a credential.

## Release packaging

```bash
scripts/package.sh
```

The script builds the release binary, creates a checksummed archive, extracts it
into a temporary directory, and smoke-tests `--version` and `config doctor`.
Signing is an external credential/release-plan gate and is never faked by the
script or workflow.
