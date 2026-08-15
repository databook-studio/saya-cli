#!/usr/bin/env bash
# Builds the throwaway shop database every memory demo records against.
# Deterministic: same rows, same schema, every run — so the GIFs are reproducible.
set -euo pipefail
ROOT="${1:?usage: _fixture.sh <dir>}"
rm -rf "$ROOT" && mkdir -p "$ROOT/home"
cd "$ROOT"
python3 - <<'PY'
import sqlite3
c = sqlite3.connect("shop.db")
c.execute("CREATE TABLE orders(id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL,"
          " created_at TEXT NOT NULL, ordered_at TEXT, status TEXT NOT NULL, total REAL NOT NULL)")
c.execute("CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT NOT NULL, tier_code TEXT)")
for i in range(1, 13):
    c.execute("INSERT INTO orders VALUES (?,?,?,?,?,?)",
              (i, i % 4 + 1, f"2026-{i:02d}-05", f"2026-{i:02d}-01", "shipped", 10.0 * i))
for i, n in enumerate(["Acme", "Globex", "Initech"], 1):
    c.execute("INSERT INTO accounts VALUES (?,?,?)", (i, n, "3"))
c.commit(); c.close()
PY
printf '[profiles.shop]\ntype = "sqlite"\npath = "shop.db"\nread_only = true\n' > connections.toml
printf '[memory]\nrecall = "confirmed"\nlearning = "off"\n' > config.toml
