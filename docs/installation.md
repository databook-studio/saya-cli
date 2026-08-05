# Installation

SAYA CLI is currently a private-alpha source release. Rust 1.88.0 or newer is
required.

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

The repository is not published on crates.io and there is no Homebrew formula.
For a future public repository, the intended source install shape is:

```bash
cargo install --locked --git https://github.com/databook-studio/saya-cli.git saya-cli
```

That command is future-facing documentation, not a currently supported release
channel. Prebuilt archives are produced only by the release-candidate workflow
after its checks pass.

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
