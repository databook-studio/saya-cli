use saya_types::ConnectionError;
use sqlx::{ConnectOptions, Connection};
use tokio::time::timeout;

use super::{MySqlConnector, errors};

/// Deadline for the dedicated `KILL QUERY` connection. Bounded independently of
/// `query_timeout` (which may be tens of seconds): the caller already holds a
/// timed-out query; the kill must not strand it for another full acquire wait.
const KILL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// Atomically claim the in-flight connection id to kill.
///
/// `take` (not a peek) makes cancellation single-winner: the first caller to
/// reach this wins the id, and a concurrent caller — a Ctrl-C from the TUI
/// racing the in-`query` cleanup — finds `None` and becomes a no-op instead of
/// issuing a second `KILL QUERY`. This is the seam Defect A hid behind: the old
/// `query` cleared `active_id` *before* calling `cancel`, so by the time
/// `cancel` looked, the id was gone and the kill never issued.
pub(crate) fn next_kill_target(active: &mut Option<u64>) -> Option<u64> {
    active.take()
}

pub(crate) async fn cancel(connector: &MySqlConnector) -> Result<(), ConnectionError> {
    let id = next_kill_target(&mut *connector.active_id.lock().await);
    let Some(id) = id else {
        // Nothing in flight: cancelling is a no-op, not an error.
        return Ok(());
    };
    // The id comes from CONNECTION_ID() as an integer and is interpolated
    // only into this KILL statement; KILL does not accept bound parameters.
    //
    // The kill runs on its own short-lived connection, not `&connector.pool`:
    // the timed-out query holds its pooled connection (see execute.rs), so a
    // pool acquire would compete with it and, under `max_connections` of 1,
    // block until `acquire_timeout` — and invariant 2 requires the kill to
    // issue before we return. A dedicated connection sidesteps pool size
    // entirely; it is bounded by `KILL_DEADLINE` so a dead server cannot
    // strand the caller.
    let work = async {
        let mut connection = connector.kill_options.connect().await?;
        sqlx::query(&format!("KILL QUERY {id}"))
            .execute(&mut connection)
            .await?;
        connection.close().await?;
        Ok::<(), sqlx::Error>(())
    };
    timeout(KILL_DEADLINE, work)
        .await
        .map_err(|_| ConnectionError::query_failed("MySQL cancellation timed out"))?
        .map_err(errors::query)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! The cancellation ordering is pure and synchronous where the kill
    //! *decision* is concerned, so we test it without a live MySQL server.
    //! The actual `KILL QUERY` round-trip needs a server (see the
    //! `SAYA_TEST_MYSQL_URL` live harness, outside these owned paths) — these
    //! tests prove the ordering invariant Defect A broke, not the wire bytes.

    use super::next_kill_target;

    /// Defect A, reproduced: the old `query` cleared `active_id` *before*
    /// calling `cancel`, so by the time cancellation claimed its target the id
    /// was already gone and the kill never issued. The fix is an ordering
    /// change in `execute.rs`, not a change here — this test pins the contract
    /// `cancel` relies on: claim from a live id yields the kill; claim from an
    /// already-cleared slot yields nothing.
    #[test]
    fn claim_uses_the_id_the_query_left_in_flight() {
        // The fixed `query` leaves the id in flight for `cancel` to claim.
        let mut live = Some(7_u64);
        assert_eq!(next_kill_target(&mut live), Some(7), "kill is issued");
        // The old `query` had already cleared the slot, so `cancel` saw this:
        let mut already_cleared: Option<u64> = None;
        assert_eq!(
            next_kill_target(&mut already_cleared),
            None,
            "pre-clearing defeats the kill (Defect A)"
        );
    }

    /// The fix is single-winner: the first `cancel` to claim consumes the id,
    /// so a concurrent Ctrl-C racing the in-`query` cleanup finds the slot
    /// empty and becomes a no-op instead of issuing a second `KILL QUERY`
    /// (the answer to Q2).
    #[test]
    fn claim_is_single_winner_no_double_kill() {
        let mut active = Some(7_u64);
        assert_eq!(next_kill_target(&mut active), Some(7), "first caller wins");
        assert_eq!(active, None, "id is consumed, not peeked");
        assert_eq!(next_kill_target(&mut active), None, "no double-kill");
        assert_eq!(active, None);
    }

    /// Cancelling with nothing in flight is a successful no-op.
    #[test]
    fn claim_with_nothing_in_flight_is_a_no_op() {
        let mut active: Option<u64> = None;
        assert_eq!(next_kill_target(&mut active), None);
        assert_eq!(active, None);
    }
}
