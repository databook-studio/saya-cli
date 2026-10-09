// ---------------------------------------------------------------------------
// harness — mirrors tests/contracts_cli.rs so both suites share one store shape
// and the derived profile identity is identical across them.
// ---------------------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-contracts-portable-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

/// A runtime over a sqlite database file. With `extra` profiles the file names
/// more than one profile; each gets its own database file so two profiles never
/// alias one schema. The connections path *is* the cache scope, so the derived
/// identity is deterministic per root.
fn runtime_with(root: &Path, extra: &[&str]) -> RuntimeConfig {
    let database = root.join("data.sqlite3");
    fs::write(&database, b"").unwrap();
    let mut connections = format!(
        "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
        database.display()
    );
    for name in extra {
        let file = root.join(format!("data-{name}.sqlite3"));
        fs::write(&file, b"").unwrap();
        connections.push_str(&format!(
            "\n[profiles.{name}]\ntype = 'sqlite'\npath = '{}'\n",
            file.display()
        ));
    }
    fs::write(root.join("connections.toml"), connections).unwrap();
    // More than one profile needs an explicit default in config.toml.
    if !extra.is_empty() {
        fs::write(root.join("config.toml"), "default_profile = \"local\"\n").unwrap();
    }
    let options = saya_cli::GlobalOptions {
        connections: Some(root.join("connections.toml")),
        ..Default::default()
    };
    load_with_sources(&options, root, root, BTreeMap::new()).unwrap()
}

fn runtime_at(root: &Path) -> RuntimeConfig {
    runtime_with(root, &[])
}

/// A store whose migrations have run, at `root/state.sqlite3`.
async fn store_at(root: &Path, runtime: &RuntimeConfig) -> SqliteStateStore {
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    // Touch the pool so migrations run, seeding an empty default cache for the
    // `local` identity exactly as `contracts_cli.rs` does.
    let identity = identity_for(runtime, "local");
    store
        .upsert_schema(&identity, &SchemaTree::default())
        .await
        .unwrap();
    store
}

fn identity_for(runtime: &RuntimeConfig, name: &str) -> String {
    let profile = runtime.named_profile(name).unwrap();
    profile_identity(name, profile, &runtime.cache_scope)
        .as_str()
        .to_owned()
}

async fn run(
    command: ContractsCommand,
    runtime: &RuntimeConfig,
    store: &SqliteStateStore,
    format: RenderFormat,
) -> (i32, String, String) {
    capture_output_start();
    let code = run_contracts(command, runtime, format, store)
        .await
        .unwrap();
    let (out, err) = capture_output_take();
    (code, out, err)
}

// ---------------------------------------------------------------------------
// schema + claim seeds
// ---------------------------------------------------------------------------
