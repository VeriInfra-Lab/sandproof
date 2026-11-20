use super::*;
use serde_json::json;

fn samples() -> Vec<AcceptedLifecycleIntent> {
    let operations = [
        json!({"kind":"create","create":{"template_revision":"template",
            "template_digest":vec![255;32],"resources":{"instances":1,"cpu":1,
            "memory_bytes":4096,"disk_bytes":8192},"options":[0,255,31],
            "startup_deadline_ms":1,"timeout_ms":10,"max_lifetime_ms":20},
            "target_incarnation":1}),
        json!({"kind":"unload","source_incarnation":1,"capture":2}),
        json!({"kind":"resume","source_incarnation":1,"capture":2,
            "target_incarnation":9,"manifest_digest":vec![255;32]}),
        json!({"kind":"renew","incarnation":9,"end_at_ms":-1,"extend_only":false}),
        json!({"kind":"destroy"}),
    ];
    operations
        .into_iter()
        .enumerate()
        .map(|(index, operation)| {
            serde_json::from_value(json!({
                "request":{"binding":{"protocol_version":4,
                    "node":{"deployment_id":"deploy","node_id":"node","node_generation":1},
                    "session":"session","key":{"deployment":"deploy","owner":"owner",
                        "window":4,"sequence":5},"expected":if index == 0 {
                            json!({"state":"absent"})
                        } else { json!({"state":"version","version":1}) }},"operation":operation},
                "accepted_version":if index == 0 {1} else {2},
                "center":{"deployment":"deploy","center":"center"},
                "retention":{"slot":1,"window":9}
            }))
            .unwrap()
        })
        .collect()
}

fn recorded(value: AcceptedLifecycleIntent, terminal: bool) -> RecordedLifecycleIntent {
    RecordedLifecycleIntent {
        current_session_version: value.accepted_version,
        intent: value,
        current: true,
        terminal,
    }
}

#[test]
fn original_full_envelopes_keep_complete_bytes_and_exact_threshold() {
    for value in samples() {
        let context = sandproof_protocol::lifecycle::AuthenticatedCenterContext::from_verified_peer_registration(
            value.center.clone(), value.request.binding.node.clone());
        assert_eq!(value.validate(&context), Ok(()));
        let expected = serde_json::to_vec(&value).unwrap();
        assert_eq!(accepted_bounded(&value, expected.len()).unwrap(), expected);
        assert_eq!(
            serde_json::from_slice::<AcceptedLifecycleIntent>(&expected).unwrap(),
            value
        );
        assert!(matches!(
            accepted_bounded(&value, expected.len() - 1),
            Err(JournalError::MaintenanceCapacity)
        ));
        for terminal in [false, true] {
            let record = recorded(value.clone(), terminal);
            let expected = serde_json::to_vec(&record).unwrap();
            assert_eq!(recorded_bounded(&record, expected.len()).unwrap(), expected);
            assert_eq!(
                serde_json::from_slice::<RecordedLifecycleIntent>(&expected).unwrap(),
                record
            );
            assert!(matches!(
                recorded_bounded(&record, expected.len() - 1),
                Err(JournalError::MaintenanceCapacity)
            ));
        }
    }
}

#[cfg(feature = "candidate")]
mod candidate {
    use super::*;
    use sandproof_controller_service::template_registry::evidence::{
        BoundedBuffer, encode_bounded_with_buffer,
    };
    use serde::Serialize;
    use std::{cell::RefCell, io};

    #[derive(Default, Clone)]
    struct Observations {
        fail_at: Option<usize>,
        reserves: usize,
        failed: bool,
        prefix: Vec<u8>,
        writes_after_failure: usize,
        moved: bool,
    }
    thread_local! {
        static OBSERVATIONS: RefCell<Observations> = RefCell::new(Observations::default());
    }
    fn reset(fail_at: Option<usize>) {
        OBSERVATIONS.set(Observations {
            fail_at,
            ..Default::default()
        });
    }
    fn observed() -> Observations {
        OBSERVATIONS.with_borrow(Clone::clone)
    }

    struct FaultBuffer {
        bytes: Vec<u8>,
        capacity: usize,
    }
    impl BoundedBuffer for FaultBuffer {
        type ReserveError = ();
        fn new_buffer() -> Self {
            Self {
                bytes: Vec::new(),
                capacity: 0,
            }
        }
        fn len(&self) -> usize {
            OBSERVATIONS.with_borrow_mut(|state| {
                if state.failed {
                    state.writes_after_failure += 1;
                }
            });
            self.bytes.len()
        }
        fn capacity(&self) -> usize {
            self.capacity
        }
        fn try_reserve_exact(&mut self, additional: usize) -> Result<(), ()> {
            let refuse = OBSERVATIONS.with_borrow_mut(|state| {
                let refuse = state.fail_at == Some(state.reserves);
                state.reserves += 1;
                if refuse {
                    state.failed = true;
                    state.prefix = self.bytes.clone();
                }
                refuse
            });
            if refuse {
                return Err(());
            }
            self.bytes.try_reserve_exact(additional).map_err(|_| ())?;
            // Deterministic lawful capacity exposes every nonempty write boundary.
            self.capacity = self.bytes.len() + additional;
            Ok(())
        }
        fn extend_from_slice(&mut self, bytes: &[u8]) {
            self.bytes.extend_from_slice(bytes);
        }
        fn into_bytes(self) -> Vec<u8> {
            OBSERVATIONS.with_borrow_mut(|state| state.moved = true);
            self.bytes
        }
        fn capacity_error() -> io::Error {
            io::Error::other("bounded evidence exceeded")
        }
        fn allocation_error() -> io::Error {
            io::Error::other("bounded evidence allocation failed")
        }
    }

    fn check_every_allocation_failure<T: Serialize>(value: &T) -> usize {
        let complete = serde_json::to_vec(value).unwrap();
        reset(None);
        assert_eq!(
            encode_bounded_with_buffer::<FaultBuffer>(value, complete.len()).unwrap(),
            complete
        );
        let count = observed().reserves;
        assert!(count > 20);
        for at in 0..count {
            reset(Some(at));
            let result = encode_bounded_with_buffer::<FaultBuffer>(value, complete.len());
            let error = match result {
                Err(JournalError::Json(error)) => error,
                other => panic!(
                    "same complete encoding contract lost the original serializer error at {at}: {other:?}"
                ),
            };
            assert!(error.is_io());
            assert_eq!(error.to_string(), "bounded evidence allocation failed");
            let state = observed();
            assert_eq!(state.reserves, at + 1);
            assert!(state.failed);
            assert_eq!(state.writes_after_failure, 0);
            assert!(!state.moved);
            assert!(complete.starts_with(&state.prefix));
            assert!(state.prefix.len() < complete.len());
        }
        count
    }

    #[test]
    fn standard_full_envelopes_stop_at_every_allocation_failure() {
        let mut points = 0;
        for value in samples() {
            points += check_every_allocation_failure(&value);
            for terminal in [false, true] {
                points += check_every_allocation_failure(&recorded(value.clone(), terminal));
            }
        }
        println!("checked original standard derives at {points} exact allocation failure points");
    }

    #[test]
    fn size_fault_precedes_reservation_and_serializer_result() {
        let value = samples().pop().unwrap();
        reset(Some(0));
        assert!(matches!(
            encode_bounded_with_buffer::<FaultBuffer>(&value, 0),
            Err(JournalError::MaintenanceCapacity)
        ));
        let state = observed();
        assert_eq!(state.reserves, 0);
        assert!(!state.failed);
        assert!(!state.moved);
    }
}
