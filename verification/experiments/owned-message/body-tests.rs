// Appended only to the isolated original command/body.rs.
#[cfg(test)]
mod owned_message_contract_tests {
    use super::super::streams::owned_message_contract_tests::{finish, fixture, publish, take};
    use super::*;
    #[tokio::test]
    async fn last_bytes_clone_keeps_output_and_original_leases() {
        let mut rig = fixture().await;
        publish(&rig.command, b"abcd");
        let output = take(&mut rig.subscription).await;
        let lease = Arc::new(Lease {
            _request: Arc::new(()),
            _subscriber: Arc::clone(&rig.subscription.slot),
            _record: Arc::clone(&rig.subscription.record),
        });
        let mut producer = Producer {
            subscription: rig.subscription,
            lease,
            limits: CommandBodyLimits {
                message_bytes: 1024,
                keepalive: Duration::from_secs(30),
            },
            deadline: Instant::now() + Duration::from_secs(30),
            released: None,
            clock: None,
            stage: Stage::Output,
        };
        let frame = producer.frame(b"encoded".to_vec(), Some(output), true);
        let mut acknowledgment = producer.released.take().unwrap().received;
        let bytes = Bytes::from_owner(frame);
        let clone = bytes.clone();
        finish(&rig.command);
        drop(producer);
        drop(rig.command);
        drop(bytes);
        assert_eq!(clone.as_ref(), b"encoded");
        assert_eq!(
            rig.bytes.available_permits(),
            4,
            "held frame released its byte permit early"
        );
        assert_eq!(
            rig.subscribers.available_permits(),
            0,
            "held frame released subscriber permit early"
        );
        assert_eq!(
            rig.records.available_permits(),
            0,
            "held frame released original record early"
        );
        assert!(matches!(
            acknowledgment.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        drop(clone);
        acknowledgment.try_recv().unwrap();
        assert_eq!(rig.bytes.available_permits(), 8);
        assert_eq!(rig.subscribers.available_permits(), 1);
        assert_eq!(rig.records.available_permits(), 1);
    }
}
