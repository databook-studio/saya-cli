//! The blocks of `02` §3.4-§3.10 as the view model holds them, and the readings
//! the pane's tests take of the rows it paints.
//!
//! One entry per block of the design, each painted through the turn painter so
//! a test asserts the block where the transcript actually reads it.

use std::time::Duration;

use super::transcript_fixture::{ELAPSED, facts, rendered};
use super::turn;
use crate::front::tui::theme::Scheme;
use crate::front::view::{
    Clarification, Command, CommandState, DiffLine, Edit, Entry, Error, Helper, HelperState,
    Notice, Stop, TurnView,
};
use crate::interactive::tui::wrap::cell_width;
use saya_agent::BudgetDimension;

/// The edit of `02` §3.4: one context row, the line it replaces, the line it
/// writes.
pub(super) fn edit() -> Edit {
    Edit {
        path: "pricing.py".to_owned(),
        lines: vec![
            DiffLine::context(14, "    discount = total * percent_off // 100"),
            DiffLine::removed(15, "    return total - discount - discount"),
            DiffLine::added(15, "    return total - discount"),
        ],
    }
}

/// A diff longer than the block shows, so the rest is counted.
pub(super) fn long_edit() -> Edit {
    Edit {
        path: "pricing.py".to_owned(),
        lines: vec![
            DiffLine::context(1, "def total(orders):"),
            DiffLine::removed(2, "    return sum(o.total for o in orders) - 10"),
            DiffLine::added(2, "    return sum(o.total for o in orders)"),
            DiffLine::context(3, ""),
            DiffLine::added(4, "def net_total(orders):"),
            DiffLine::added(5, "    return total(orders)"),
        ],
    }
}

/// The command of `02` §3.5.
pub(super) fn command() -> Command {
    Command {
        command: "python3 -m unittest".to_owned(),
        state: CommandState::Finished {
            elapsed: Duration::from_millis(400),
        },
    }
}

/// The same command as the event stream writes it: a `cd` into the workspace
/// root, then the command.
pub(super) fn command_under_root() -> Command {
    Command {
        command: "cd ~/shop && python3 -m unittest".to_owned(),
        state: CommandState::Finished {
            elapsed: Duration::from_millis(400),
        },
    }
}

/// The helper of `02` §3.6.
pub(super) fn helper() -> Helper {
    Helper {
        id: "explore-1".to_owned(),
        question: "find where pricing is tested".to_owned(),
        steps: 3,
        state: HelperState::Done,
    }
}

/// The error of `02` §3.7.
pub(super) fn error() -> Error {
    Error {
        message: "Provider error — rate limited (429). Retried 3 times.".to_owned(),
    }
}

/// The budget stop of `02` §3.8.
pub(super) fn budget() -> Stop {
    Stop::Budget {
        dimension: BudgetDimension::ElapsedTime,
        limit: Some("[limits.database] elapsed_seconds = 300".to_owned()),
    }
}

/// The pause of `02` §3.8.
pub(super) fn paused() -> Stop {
    Stop::Paused {
        message: "no progress in 8 turns (limits.job.no_progress_turns).".to_owned(),
    }
}

/// The operator's cancel of `02` §3.8.
pub(super) fn cancelled() -> Stop {
    Stop::Cancelled
}

/// The question of `02` §3.9.
pub(super) fn question() -> Clarification {
    Clarification {
        question: "Which revenue do you mean?".to_owned(),
        options: vec![
            "Gross order value (orders.total)".to_owned(),
            "Net of refunds (orders.total − refunds.amount)".to_owned(),
            "Recognised revenue (ledger.revenue)".to_owned(),
        ],
    }
}

/// The session's own line of `02` §3.10.
pub(super) fn system() -> Notice {
    Notice {
        message: "mode: accept-edits".to_owned(),
    }
}

/// A turn holding only `entry`.
pub(super) fn turn_of(entry: Entry) -> TurnView {
    let mut turn = TurnView::default();
    turn.push(entry);
    turn
}

/// One entry as the turn painter paints it, in the design's dark scheme.
pub(super) fn painted(entry: Entry, width: usize) -> Vec<String> {
    painted_in(entry, width, Scheme::Dark)
}

/// One entry as the turn painter paints it, in `scheme`.
pub(super) fn painted_in(entry: Entry, width: usize, scheme: Scheme) -> Vec<String> {
    let mut held = facts(ELAPSED);
    held.scheme = scheme;
    rendered(&turn::rows(&turn_of(entry), ELAPSED, &held, width))
}

/// Every block of `02` §3.4-§3.10, by the name a failure should mention.
pub(super) fn every_block() -> Vec<(&'static str, Entry)> {
    vec![
        ("edit", Entry::Edit(edit())),
        ("long edit", Entry::Edit(long_edit())),
        ("command", Entry::Command(command())),
        ("helper", Entry::Helper(helper())),
        ("error", Entry::Error(error())),
        ("budget stop", Entry::Stop(budget())),
        ("pause", Entry::Stop(paused())),
        ("cancel", Entry::Stop(cancelled())),
        ("question", Entry::Question(question())),
        ("system line", Entry::Notice(system())),
    ]
}

/// The cells one painted row takes.
pub(super) fn cells_of(row: &str) -> usize {
    cell_width(row)
}
