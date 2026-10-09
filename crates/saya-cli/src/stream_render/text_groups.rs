/// Append-only text groups collapse by buffer-then-decide: tool events
/// accumulate from group-open until the next boundary event, then the shaped
/// summary flushes where the group's first line would have printed. The
/// buffer is bounded — a force-flush every [`GROUP_CALL_BOUND`] completed
/// calls degrades a runaway group into chunk summaries, never unbounded
/// memory. Only the `Text` adapter groups; `Json`/`Ndjson` bypass entirely.
const GROUP_CALL_BOUND: usize = 32;

/// The pending run the text adapter has buffered but not yet decided on.
/// `completed` counts completed calls purely to bound the buffer (see
/// `GROUP_CALL_BOUND`); rendering decisions come from the shared grouper at
/// flush time, never from this counter.
#[derive(Debug, Default)]
struct TextGroupState {
    pending: Vec<AgentEvent>,
    completed: usize,
}

impl TextGroupState {
    fn new() -> Self {
        Self::default()
    }
}

fn is_group_member(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::ToolRequested { .. } | AgentEvent::ToolCompleted { .. }
    )
}

/// The stream entry point: `Text` groups tool runs through the shared
/// grouper before rendering; `Json`/`Ndjson` bypass the grouper entirely so
/// the machine surface stays event-for-event. The boundary rule mirrors the
/// grouper exactly — every non-member event flushes the open group, including
/// silent ones — so the pipe renders the shared grouping, never a second one.
fn render_text_stream(
    event: AgentEvent,
    format: RenderFormat,
    text_open: &mut bool,
    group: &mut TextGroupState,
) -> Rendered {
    if !matches!(format, RenderFormat::Text) {
        return render_agent(event, format, text_open);
    }
    if is_group_member(&event) {
        let completed_call = matches!(event, AgentEvent::ToolCompleted { .. });
        group.pending.push(event);
        if completed_call {
            group.completed += 1;
        }
        if group.completed >= GROUP_CALL_BOUND {
            return flush_group(group, text_open);
        }
        return Rendered {
            stdout: String::new(),
            stderr: String::new(),
        };
    }
    let mut out = flush_group(group, text_open);
    let rendered = render_agent(event, format, text_open);
    out.stdout.push_str(&rendered.stdout);
    out.stderr.push_str(&rendered.stderr);
    out
}

/// Shapes the buffered run into the Decision-2 summary (or today's verbatim
/// lines for a single call) and renders it through the text adapter,
/// preserving the assistant-delta close the ungrouped path applies.
fn flush_group(group: &mut TextGroupState, text_open: &mut bool) -> Rendered {
    if group.pending.is_empty() {
        return Rendered {
            stdout: String::new(),
            stderr: String::new(),
        };
    }
    let pending = std::mem::take(&mut group.pending);
    group.completed = 0;
    let mut out = Rendered {
        stdout: String::new(),
        stderr: String::new(),
    };
    let groups = crate::render::tool_groups::group_tool_events(&pending);
    for shaped in &groups {
        for line in crate::render::tool_groups::shape_group(shaped) {
            let line = crate::render::sanitize_terminal(&line);
            if *text_open {
                out.stdout.push('\n');
                *text_open = false;
            }
            out.stdout.push_str(&line);
            if !line.ends_with('\n') {
                out.stdout.push('\n');
            }
        }
    }
    out
}

fn render_agent(event: AgentEvent, format: RenderFormat, text_open: &mut bool) -> Rendered {
    let Some(event) = terminal_event(event) else {
        // Nothing to print, and nothing to close: a progress signal must not
        // terminate an open text block the way a real event would.
        return Rendered {
            stdout: String::new(),
            stderr: String::new(),
        };
    };
    let close = matches!(format, RenderFormat::Text)
        && *text_open
        && !matches!(event, TerminalEvent::AssistantText { .. });
    let mut rendered = render_event(&event, format);
    if close && !matches!(event, TerminalEvent::Complete) {
        rendered.stdout.insert(0, '\n');
        *text_open = false;
    }
    match event {
        TerminalEvent::AssistantText { ref text } if matches!(format, RenderFormat::Text) => {
            *text_open = !text.is_empty()
        }
        TerminalEvent::Complete if matches!(format, RenderFormat::Text) => {
            *text_open = false;
            if !close {
                rendered.stdout.clear();
            }
        }
        _ => {}
    }
    rendered
}
