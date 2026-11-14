# Installation

SAYA CLI ships prebuilt binaries for macOS (Apple Silicon + Intel), Linux
(x86_64), and Windows (x86_64) on every
[release](https://github.com/databook-studio/saya-cli/releases), and is
published on crates.io. Building from source requires Rust 1.89.0 or newer.

## Install a release build

```bash
# Homebrew (macOS / Linux)
brew install databook-studio/tap/saya

# Cargo, prebuilt binary, no compile (https://github.com/cargo-bins/cargo-binstall)
cargo binstall saya-cli

# Cargo, from source (allow a few minutes)
cargo install saya-cli
```

All of these install a single `saya` binary onto your `PATH`.

## Git

Jobs need Git 2.34 or newer. Ubuntu 22.04 LTS (Git 2.34.1) and Debian 12
(Git 2.39) are supported. Saya adapts to the installed version: `--no-lazy-fetch`
and `GIT_NO_LAZY_FETCH` are used from Git 2.45, and `git worktree list
--porcelain -z` from Git 2.36; older Git uses the newline-terminated listing.
Once Saya detects Git 2.45 or newer, it also sets `GIT_NO_LAZY_FETCH` on the
Git processes it spawns. A partial clone (a promisor remote or
`extensions.partialClone`) is refused on every supported version because Saya
cannot materialize missing objects; below 2.45 that refusal is also the only
available protection against lazy fetching. An ordinary repository proceeds.
`saya job doctor` reports the detected version and the compatibility paths in
use.

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

The fastest first run needs no configuration at all: `saya demo` builds a
small synthetic SQLite database and opens it read-only in the interactive
session, where schema browsing and `/sql` work without any AI provider.

```bash
saya demo
```

To point saya at your own database, `saya setup` walks you through choosing
an AI provider and a database profile, probing the connection, and
confirming the exact file changes (secrets are requested only as
environment-variable names). All six engines are guided — SQLite,
PostgreSQL, MySQL, Snowflake (keypair, browser SSO, or password-env),
ClickHouse, and BigQuery. If you prefer to write the files yourself, the
manual path is unchanged:

```bash
saya config init
${EDITOR:-vi} ~/.config/saya/config.toml
${EDITOR:-vi} ~/.config/saya/connections.toml
export SAYA_ANALYTICS_PASSWORD='use-a-read-only-password'
saya config doctor
saya connection test analytics
saya --profile analytics --approval-mode read-only query --sql 'SELECT 1'
```

`config init` writes to your user config directory (`--project` writes the
repo's `.saya/` pair instead). It refuses to overwrite either file, uses restrictive Unix modes,
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
