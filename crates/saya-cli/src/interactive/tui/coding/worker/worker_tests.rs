use super::{CodingMessage, deliver_outcome};

#[test]
fn outcome_is_delivered_even_when_the_activity_queue_is_full() {
    let (activity, activity_rx) = std::sync::mpsc::sync_channel::<CodingMessage>(2);
    let (receipts, receipts_rx) = std::sync::mpsc::sync_channel::<CodingMessage>(2);
    assert!(
        activity
            .try_send(CodingMessage::Activity("progress".into()))
            .is_ok()
    );
    assert!(
        activity
            .try_send(CodingMessage::Activity("progress".into()))
            .is_ok()
    );
    assert!(
        activity
            .try_send(CodingMessage::Activity("overflow".into()))
            .is_err()
    );
    deliver_outcome(&receipts, "receipt-text".into());
    // A full activity queue must not hold a control receipt: receipts ride
    // their own queue, so the outcome is available immediately and the
    // panel's next drain pushes it after the activity backlog.
    assert!(activity_rx.try_recv().is_ok());
    assert!(matches!(
        receipts_rx.try_recv(),
        Ok(CodingMessage::Activity(text)) if text == "receipt-text"
    ));
    assert!(receipts_rx.try_recv().is_err());
    drop(activity_rx);
    drop(receipts_rx);
}

mod approval_scope {
    use super::super::{CodingMessage, PanelApproval};
    use saya_agent::{ApprovalDecider, CancellationToken};
    use serde_json::json;

    fn panel(id: &str) -> (PanelApproval, std::sync::mpsc::Receiver<CodingMessage>) {
        let (sender, rx) = std::sync::mpsc::sync_channel(4);
        let approval = PanelApproval {
            sender,
            token: CancellationToken::new(),
            id: id.into(),
        };
        (approval, rx)
    }

    #[tokio::test]
    async fn a_proposal_for_another_job_is_refused_without_asking_the_operator() {
        let (approval, rx) = panel("job-a");
        let other = json!({"operation":"start","job_id":"job-b"});
        let tool = crate::commands::coding::CodingAuthorization::definition();
        assert!(!approval.approve(&tool, &other).await);
        assert!(
            rx.try_recv().is_err(),
            "no approval prompt may be shown for a job this panel does not own"
        );
        assert!(!approval.approve(&tool, &json!({"operation":"start"})).await);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_proposal_for_this_job_prompts_and_follows_the_answer() {
        let (approval, rx) = panel("job-a");
        let tool = crate::commands::coding::CodingAuthorization::definition();
        let proposal = json!({"operation":"start","job_id":"job-a"});
        let asked = tokio::spawn(async move { approval.approve(&tool, &proposal).await });
        let message = loop {
            match rx.try_recv() {
                Ok(message) => break message,
                Err(_) => tokio::task::yield_now().await,
            }
        };
        let CodingMessage::Approval { respond, .. } = message else {
            panic!("the operator is asked through the approval message");
        };
        respond.send(true).unwrap();
        assert!(asked.await.unwrap());
    }
}
