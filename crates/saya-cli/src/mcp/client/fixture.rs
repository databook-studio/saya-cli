//! A tiny stdio MCP server, written as a shell script, for the client's tests.
//!
//! A real server would be another process to build and a second protocol
//! implementation to trust. This one is a `sh` loop that answers the three
//! requests the client makes and can be told to misbehave in each of the ways
//! the client's bounds exist for: a hanging server, one that exits at once, one
//! whose descriptions or schemas are oversized, one that lists in pages, one
//! that floods a frame, one that sends a notification before every reply.
//!
//! The mode is read from `FIXTURE_MODE`, so a test states the misbehaviour in
//! the server's own configuration (`env`) rather than in the fixture's code.

use std::path::{Path, PathBuf};

/// The script, verbatim. It reads one JSON-RPC request per line and answers it.
const SERVER: &str = r#"#!/bin/sh
# A minimal stdio MCP server. One JSON-RPC request per line on stdin, one
# reply per line on stdout. FIXTURE_MODE selects a misbehaviour.
mode="${FIXTURE_MODE:-ok}"

reply() {
    # $1 = id, $2 = result JSON
    printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$1" "$2"
}

tools_ok='{"tools":[{"name":"echo","description":"Echo the call back.","inputSchema":{"type":"object","properties":{"text":{"type":"string"}}}},{"name":"fail","description":"Report the call as failed.","inputSchema":{"type":"object"}},{"name":"secret","description":"Print the token this server was handed.","inputSchema":{"type":"object"}}]}'
tools_big_desc='{"tools":[{"name":"echo","description":"DESCRIPTION","inputSchema":{"type":"object"}}]}'
tools_big_schema='{"tools":[{"name":"echo","description":"fine","inputSchema":{"type":"object","pad":"SCHEMA"}}]}'
tools_bad_names='{"tools":[{"name":"has space","description":"d","inputSchema":{"type":"object"}},{"name":"ok_name","description":"d","inputSchema":{"type":"object"}}]}'

if [ "$mode" = "exit" ]; then
    exit 3
fi

while IFS= read -r line; do
    id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
    case "$line" in
        *'"method":"initialize"'*)
            if [ "$mode" = "hang" ]; then
                sleep 60
                continue
            fi
            reply "$id" '{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"fixture","version":"1"}}'
            ;;
        *'"method":"notifications/initialized"'*)
            :
            ;;
        *'"method":"tools/list"'*)
            if [ "$mode" = "huge_frame" ]; then
                # One line far past any frame cap: the client must discard it
                # as it arrives rather than buffer it.
                printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"' "$id"
                head -c 8000000 /dev/zero | tr '\0' 'x'
                printf '"}]}}\n'
                continue
            fi
            if [ "$mode" = "many_tools" ]; then
                printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[' "$id"
                i=0
                while [ $i -lt 200 ]; do
                    if [ $i -gt 0 ]; then printf ','; fi
                    printf '{"name":"tool_%s","description":"d","inputSchema":{"type":"object"}}' "$i"
                    i=$((i + 1))
                done
                printf ']}}\n'
                continue
            fi
            if [ "$mode" = "paginate" ] && [ -z "$cursor_done" ]; then
                cursor_done=1
                reply "$id" '{"tools":[{"name":"first","description":"d","inputSchema":{"type":"object"}}],"nextCursor":"page-2"}'
                continue
            fi
            case "$mode" in
                big_description) reply "$id" "$tools_big_desc" ;;
                big_schema) reply "$id" "$tools_big_schema" ;;
                bad_names) reply "$id" "$tools_bad_names" ;;
                *) reply "$id" "$tools_ok" ;;
            esac
            ;;
        *'"method":"tools/call"'*)
            tool=$(printf '%s' "$line" | sed -n 's/.*"name":"\([^"]*\)".*/\1/p')
            escaped=$(printf '%s' "$line" | sed 's/\\/\\\\/g; s/"/\\"/g')
            if [ "$mode" = "notify_first" ]; then
                printf '{"jsonrpc":"2.0","method":"notifications/message","params":{"level":"info"}}\n'
            fi
            if [ "$tool" = "fail" ]; then
                reply "$id" '{"content":[{"type":"text","text":"the tool refused"}],"isError":true}'
                continue
            fi
            if [ "$tool" = "secret" ]; then
                reply "$id" "{\"content\":[{\"type\":\"text\",\"text\":\"token is ${FIXTURE_TOKEN:-none}\"}],\"isError\":false}"
                continue
            fi
            reply "$id" "{\"content\":[{\"type\":\"text\",\"text\":\"echo $escaped\"}],\"isError\":false}"
            ;;
        *'"method":"notifications/'*)
            :
            ;;
        *)
            printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32601,"message":"unknown method"}}\n' "$id"
            ;;
    esac
done
"#;

/// Writes the fixture server into `dir` and returns its path.
pub(crate) fn server_script(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).expect("the fixture directory is creatable");
    let path = dir.join("mcp-fixture-server.sh");
    std::fs::write(&path, SERVER).expect("the fixture server is writable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("the fixture server is made executable");
    }
    path
}

/// A directory for one test's fixture, removed first so a rerun starts clean.
pub(crate) fn dir(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-mcp-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the fixture directory is creatable");
    root
}

/// A description long enough to be truncated, for the oversized-description
/// fixture mode. The script substitutes it for `DESCRIPTION`.
pub(crate) fn oversized_description(script: &Path) {
    let body = std::fs::read_to_string(script).expect("the fixture server is readable");
    let long = "d".repeat(4_000);
    std::fs::write(script, body.replace("DESCRIPTION", &long))
        .expect("the fixture server is writable");
}

/// A schema long enough to be dropped, for the oversized-schema fixture mode.
pub(crate) fn oversized_schema(script: &Path) {
    let body = std::fs::read_to_string(script).expect("the fixture server is readable");
    let long = "s".repeat(64 * 1024);
    std::fs::write(script, body.replace("SCHEMA", &long)).expect("the fixture server is writable");
}
