struct ScratchConfig {
    config: PathBuf,
    connections: PathBuf,
}

/// Whether the scratch config turns `[memory]` on. `Off` writes no `[memory]`
/// table (the original smoke test); `Assisted` writes `[memory] mode =
/// 'assisted'` — the section every failing demo tape ran.
#[derive(Copy, Clone)]
enum Memory {
    Off,
    Assisted,
}

/// Writes a minimal, offline config + connections file under `home`. No
/// `--env-file`, no provider key, and a single `demo` profile pointing at a
/// scratch SQLite file that is never opened — the test sends no question. The
/// profile name leads the status bar as `demo · smoke`. When `memory` is
/// `Assisted` the config adds `[memory] mode = 'assisted'`, the suspect path
/// for defect #51; `Off` keeps the original no-`[memory]` shape.
fn write_scratch_config(home: &Path, memory: Memory) -> ScratchConfig {
    let config = home.join("config.toml");
    let memory_section = match memory {
        Memory::Off => String::new(),
        Memory::Assisted => "\n[memory]\nmode = 'assisted'\n".to_string(),
    };
    std::fs::write(
        &config,
        // [ai] model only; no provider key. Loads offline. The optional
        // [memory] section is the only difference between the rungs.
        format!("[ai]\nmodel = 'smoke'\n\n[run]\nmax_rows = 10\n{memory_section}"),
    )
    .unwrap();

    let database = home.join("demo.sqlite3");
    let connections = home.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.demo]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();

    ScratchConfig {
        config,
        connections,
    }
}

/// The docker pagila profile, mirrored from `.saya/connections.toml` so rung 3
/// does not depend on the developer's checked-in file (the test owns its
/// scratch HOME). The password is an env reference; `run_saya_schema_refresh`
/// sets the env value, never inlining it.
const DOCKER_POSTGRES_PASSWORD_ENV: &str = "SAYA_DOCKER_POSTGRES_PASSWORD";

/// Writes a config + connections pointing at the local docker pagila profile
/// (`docker_postgres`, port 5434) with memory on. The model is the offline
/// `smoke` placeholder — the splash and the schema refresh never call the
/// provider — and the password stays an env reference, never inlined. Used by
/// rung 3 only.
fn write_docker_config(home: &Path) -> ScratchConfig {
    let config = home.join("config.toml");
    std::fs::write(
        &config,
        // Memory on, default profile = docker_postgres. No provider key: the
        // splash is offline and `connection schema --refresh` reads no model.
        "[ai]\nmodel = 'smoke'\n\n[run]\nmax_rows = 10\n\n\
         default_profile = 'docker_postgres'\n\n[memory]\nmode = 'assisted'\n",
    )
    .unwrap();

    let connections = home.join("connections.toml");
    std::fs::write(
        &connections,
        // Mirrors .saya/connections.toml's docker_postgres profile. The
        // password is an env reference to DOCKER_POSTGRES_PASSWORD_ENV (the
        // env var name, not the Rust const) — run_saya_schema_refresh sets
        // that env var in the child; the value is never inlined in config.
        format!(
            "[profiles.docker_postgres]\n\
             type = 'postgresql'\n\
             host = '127.0.0.1'\n\
             port = 5434\n\
             database = 'pagila'\n\
             user = 'databook'\n\
             password = {{ env = '{DOCKER_POSTGRES_PASSWORD_ENV}' }}\n\
             sslmode = 'disable'\n"
        ),
    )
    .unwrap();

    ScratchConfig {
        config,
        connections,
    }
}
