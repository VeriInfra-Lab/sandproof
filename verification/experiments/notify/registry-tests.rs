// Added only in an isolated copy of the original Record module.
// This tests original field ownership, not the publish/run business policy.
#[cfg(test)]
mod notify_record_trial {
    use super::*;
    use std::future::{Future, poll_fn};
    use std::task::Poll;

    #[tokio::test]
    async fn retirement_notice_does_not_release_the_original_record_slot() {
        let registry = PidRegistry::new(
            Binding::new([1; 32]).unwrap(),
            1,
            RegistryLimits {
                records: 1,
                metadata_bytes: 1024,
                retention: Duration::from_secs(30),
            },
        )
        .unwrap();
        let reservation = registry.reserve_optional(None, None).unwrap();
        let record = Arc::new(Record {
            identity: CommandIdentity {
                session: 1,
                command: 2,
                pid: 3,
                start_ticks: 4,
            },
            metadata: reservation.metadata,
            state: Mutex::new(State::default()),
            retired: Notify::new(),
            _slot: reservation.slot,
        });
        let held = Arc::clone(&record);
        assert_eq!(registry.available_slots(), 0);
        record.retired.notify_one();
        let mut notified = Box::pin(record.retired.notified());
        assert!(
            poll_fn(|cx| Poll::Ready(notified.as_mut().poll(cx)))
                .await
                .is_ready()
        );
        drop(notified);
        assert_eq!(
            registry.available_slots(),
            0,
            "notification fabricated capacity"
        );
        drop(record);
        assert_eq!(
            registry.available_slots(),
            0,
            "another original record holder still exists"
        );
        drop(held);
        assert_eq!(registry.available_slots(), 1);
    }
}
