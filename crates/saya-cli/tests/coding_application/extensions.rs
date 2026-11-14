//! Trusted LSP fixture configuration shared by native coding journeys.

use serde_json::json;

pub(crate) fn extension_config(server: &str, selector: bool) -> String {
    let selector = if selector {
        "[job]\nautomatic_lsp_profile = 'local'\n"
    } else {
        ""
    };
    format!(
        "{selector}[job.extensions.local]\nprotocol = 'lsp'\nprogram = 'bash'\nargv = ['-c', {}]\nlanguage = 'rust'\n",
        serde_json::to_string(server).unwrap()
    )
}

pub(crate) fn lsp_server(
    stale_after_open: bool,
    malformed_diagnostics: bool,
    wait_for_release: bool,
) -> String {
    const READ_FRAME: &str = r#"
read_frame() {
    IFS= read -r header || exit 0
    length=DOLLAR{header#Content-Length: }
    length=DOLLAR{length%?}
    IFS= read -r blank || exit 0
    IFS= read -r -n "$length" body || exit 0
}
"#;
    let initialize = json!({"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}});
    let shutdown = json!({"jsonrpc":"2.0","id":2,"result":null});
    let diagnostics = if malformed_diagnostics {
        json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"__URI__","version":1,"diagnostics":[{"message":"invalid diagnostic"}]}})
    } else {
        json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"__URI__","version":1,"diagnostics":[{"range":{"start":{"line":0,"character":0}},"message":"fixture diagnostic","severity":1}]}})
    }
    .to_string();
    let (before_uri, after_uri) = diagnostics.split_once("__URI__").unwrap();
    let mut script = format!(
        "{READ_FRAME} read_frame; printf 'Content-Length: {}\\r\\n\\r\\n%s' '{}'; read_frame; read_frame; uri=DOLLAR{{body#*\\\"uri\\\":\\\"}}; uri=DOLLAR{{uri%%\\\"*}};",
        initialize.to_string().len(),
        initialize
    );
    if stale_after_open {
        let stale_gate = r#"document=DOLLAR{uri#file://}; root=DOLLAR{document%/*}; ready="DOLLARroot/.post-edit-stale-ready"; release="DOLLARroot/.post-edit-stale-release"; printf 'ready\n' > "DOLLARready"; while [ ! -f "DOLLARrelease" ]; do :; done;"#
            .replace("DOLLAR", "$");
        script.push_str(&stale_gate);
    }
    if wait_for_release {
        let wait = r#"document=DOLLAR{uri#file://}; root=DOLLAR{document%/*}; marker="DOLLARroot/.post-edit-release"; while [ ! -f "DOLLARmarker" ]; do :; done; printf 'exchange reached\n' > "DOLLARroot/.post-edit-exchange";"#
            .replace("DOLLAR", "$");
        script.push_str(&wait);
    }
    let response = format!(
        r#"payload='{}'"DOLLARuri"'{}'; printf 'Content-Length: %s\r\n\r\n%s' "DOLLAR{{#payload}}" "DOLLARpayload"; read_frame; printf 'Content-Length: {}\r\n\r\n%s' '{}'; read_frame;"#,
        before_uri,
        after_uri,
        shutdown.to_string().len(),
        shutdown
    )
    .replace("DOLLAR", "$");
    script.push_str(&response);
    script.replace("DOLLAR", "$")
}
