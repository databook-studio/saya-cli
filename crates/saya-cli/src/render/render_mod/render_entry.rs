#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub stdout: String,
    pub stderr: String,
}

pub fn render_event(event: &TerminalEvent, format: RenderFormat) -> Rendered {
    match format {
        RenderFormat::Text => text_event(event),
        RenderFormat::Json | RenderFormat::Ndjson => render_json::render(event),
    }
}

fn resume_effect_label(effect: ResumeEffectCode) -> &'static str {
    match effect {
        ResumeEffectCode::WorkspaceWrite => "workspace mutation",
        ResumeEffectCode::Scratch => "scratch database mutation",
        ResumeEffectCode::Runner => "runner execution",
        ResumeEffectCode::Interpreter => "interpreter execution",
        ResumeEffectCode::Fetch => "fetch or download",
        ResumeEffectCode::ExternalSideEffect => "external side effect",
        ResumeEffectCode::LocalStateWrite => "local state mutation",
        ResumeEffectCode::Unknown => "unknown tool effect",
    }
}

pub(crate) fn sanitize_terminal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' | '\t' => out.push(c),
            '\x00'..='\x1F' | '\x7F' | '\u{0080}'..='\u{009F}' => {}
            _ => out.push(c),
        }
    }
    out
}

/// Shapes the `ToolRequested` line: the one a pipe reader sees *before* a tool
/// runs, naming what the call may do to their machine. The `read-only` claim
/// is derived from the tool's **declared effect** through
/// [`read_only_permits`] — the same predicate the approval policy gates on —
/// so the line and the gate cannot drift: a workspace-write tool (including
/// `scratch_sql`, which declares the same effect), a network-reaching tool,
/// or any future effect that fails the predicate is never announced as
/// read-only.
///
/// The other half of the wording is a deliberate non-claim: when the
/// declaration does not prove read-only, the line says only "Using tool".
/// A positive per-effect label ("writing the workspace", "network access")
/// would need a match over the effect variants kept current by hand, and its
/// default arm would be exactly the defect this fixes — a new variant silently
/// rendering under the wrong claim. "Read-only" is the one claim the declared
/// effect proves cheaply, and only that one is made; the detail line (the SQL,
/// the path) and the tool's name carry what the call actually does.
fn tool_requested_text(name: &str, detail: Option<&str>, effect: Option<&ToolEffect>) -> String {
    let head = if effect.is_some_and(read_only_permits) {
        format!("Using read-only tool: {name}\n")
    } else {
        format!("Using tool: {name}\n")
    };
    match detail {
        Some(detail) => format!("{head}  {detail}\n"),
        None => head,
    }
}
