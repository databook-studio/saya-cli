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
    }
}

/// A call with no session grant to offer: nothing pre-answers it.
pub(crate) fn ungranted_question() -> Question {
    Question {
        grant: None,
        ..shell_question()
    }
}

/// The plan gate's ask: the plan itself as the body, and no token, because no
/// grant pre-answers a plan.
pub(crate) fn plan_question() -> Question {
    Question {
        id: 10,
        caller: plan_gate::line(&Caller::Session),
        facts: "1. Read the orders schema\n2. Count December orders\n3. Chart them by month"
            .to_owned(),
        grant: None,
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
