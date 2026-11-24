//! The questions the card's tests lay out: a SQL call, a shell call, a
//! helper's question, the plan gate's ask and a clarification — each with the
//! caller line and the facts body the session operator builds for it
//! (`chat::operator`), so a test states the bytes the operator would read.

use saya_agent::PermissionMode;
use saya_types::{AgentId, JobId};

use crate::agent::tools::coding::plan_gate;
use crate::agent::tools::consent::Caller;
use crate::chat::Question;

/// The design's reference screen: 80 columns by 24 rows.
pub(crate) const SCREEN_ROWS: u16 = 24;

/// A read-only query, as the operator puts it: the connection it runs on, then
/// the statement line for line.
pub(crate) fn sql_question() -> Question {
    Question {
        id: 7,
        caller: Caller::Session.line("Run a read-only query", "run a read-only query"),
        facts: "on: demo\nSELECT COUNT(*) AS orders_since_december\n  FROM orders\n WHERE order_date >= '2025-12-01'"
            .to_owned(),
        grant: Some("sql:demo".to_owned()),
        ..Default::default()
    }
}

/// A host command, as the operator puts it: the working directory, then the
/// command as typed.
pub(crate) fn shell_question() -> Question {
    Question {
        id: 8,
        caller: Caller::Session.line("Run a command", "run a command"),
        facts: "in: ~/shop\n$ python3 -m unittest".to_owned(),
        grant: Some("shell:python3".to_owned()),
        ..Default::default()
    }
}

/// A helper's own question: its caller line names the helper and its job.
pub(crate) fn helper_question() -> Question {
    Question {
        id: 9,
        caller: Caller::Helper {
            agent_id: helper_id(),
            job_id: job_id(),
        }
        .line("Run a shell command", "run a shell command"),
        facts: "in: ~/shop\n$ cargo test".to_owned(),
        grant: Some("shell:cargo".to_owned()),
        ..Default::default()
    }
}

/// A call with no session grant to offer: nothing pre-answers it.
pub(crate) fn ungranted_question() -> Question {
    Question {
        grant: None,
        ..shell_question()
    }
}

/// A workspace write, as the operator puts it: the path the call touches.
pub(crate) fn write_question() -> Question {
    Question {
        id: 11,
        caller: Caller::Session.line("Change a workspace file", "change a workspace file"),
        facts: "path: src/pricing.py".to_owned(),
        grant: Some("workspace-write".to_owned()),
        ..Default::default()
    }
}

/// The same write with the change the call states: an anchored replace of one
/// line, as `chat::operator` builds it from the call's own bytes (UI5).
pub(crate) fn change_question() -> Question {
    Question {
        change: change_rows(serde_json::json!({
            "path": "src/pricing.py",
            "old_text": "    return total - discount - discount",
            "new_text": "    return total - discount",
        })),
        ..write_question()
    }
}

/// A change longer than the panel's body: twelve rows written into a new file.
pub(crate) fn long_change_question() -> Question {
    let content = (0..12)
        .map(|line| format!("  row {line:02} of the new file\n"))
        .collect::<String>();
    Question {
        change: change_rows(serde_json::json!({"path": "src/new.py", "content": content})),
        facts: "path: src/new.py".to_owned(),
        ..write_question()
    }
}

/// A change that is not text, stated as its size rather than as bytes.
pub(crate) fn binary_change_question() -> Question {
    Question {
        change: change_rows(serde_json::json!({"path": "logo.png", "content": "abc\u{0}def"})),
        facts: "path: logo.png".to_owned(),
        ..write_question()
    }
}

/// The change rows of one call, exactly as the panel is handed them.
fn change_rows(arguments: serde_json::Value) -> Vec<String> {
    crate::change_diff::of(&arguments)
        .expect("the fixture's call states a change")
        .rows
}

/// A page fetch, as the operator puts it: the URL it reaches.
pub(crate) fn web_question() -> Question {
    Question {
        id: 12,
        caller: Caller::Session.line("Fetch a web page", "fetch a web page"),
        facts: "GET https://example.invalid/pricing".to_owned(),
        grant: Some("fetch:example.invalid".to_owned()),
        ..Default::default()
    }
}

/// A coding job's start consent (TX5): the scope it asks for, as one body —
/// the human lines, then the proposal's own bytes on the last line, exactly as
/// `interactive::tui::coding::consent` states them.
pub(crate) fn scope_question() -> Question {
    Question {
        id: 13,
        caller: String::from("pty-job"),
        facts: "Approve job pty-job with this exact scope:\ntask: tighten the pricing tests\n\
                write: a.txt, b.txt\nUse `/job approve pty-job` or `/job deny pty-job`.\n\
                {\"job_id\":\"pty-job\",\"operation\":\"start\",\"plan_sha256\":\"00\"}"
            .to_owned(),
        grant: None,
        ..Default::default()
    }
}

/// A path long enough to pass any panel's width: one token, so a wrap can only
/// break it where the cells run out.
pub(crate) fn long_path_question() -> Question {
    Question {
        facts: format!("path: src/{}/pricing.py", "component/".repeat(20)),
        ..write_question()
    }
}

/// The folder the launch bound, as the panel is handed it.
pub(crate) const FOLDER: Option<&str> = Some("~/shop");

/// A launch that bound no workspace.
pub(crate) const NO_FOLDER: Option<&str> = None;

/// The plan gate's ask: the plan itself as the body, and no token, because no
/// grant pre-answers a plan.
pub(crate) fn plan_question() -> Question {
    Question {
        id: 10,
        caller: plan_gate::line(&Caller::Session),
        facts: "1. Read the orders schema\n2. Count December orders\n3. Chart them by month"
            .to_owned(),
        grant: None,
        ..Default::default()
    }
}

/// A statement long enough to run past the card's six rows: twelve lines.
pub(crate) fn long_question() -> Question {
    let facts = (0..12)
        .map(|line| format!("  row {line:02} of the statement"))
        .collect::<Vec<_>>()
        .join("\n");
    Question {
        facts,
        ..sql_question()
    }
}

/// The mode a card is laid out under in the tests: the one that asks.
pub(crate) const ASK: PermissionMode = PermissionMode::Ask;

/// The helper the helper's question names.
fn helper_id() -> AgentId {
    AgentId::parse("helper-2").expect("the fixture's helper id parses")
}

/// The job the helper works for.
fn job_id() -> JobId {
    JobId::parse("job-a").expect("the fixture's job id parses")
}
