use super::*;

#[tokio::test]
async fn registry_capacity_reply_obeys_identity_gzip_bounds_and_original_owner() {
    check_capacity_reply(false).await;
    check_capacity_reply(true).await;
}

async fn check_capacity_reply(registration: bool) {
    for gzip in [false, true] {
        let make = || {
            let (mut source, slots) = source("unused", gzip);
            source.rejection = Some(if registration {
                Rejection::RegistrationCapacity
            } else {
                Rejection::RegistryCapacity
            });
            (source, slots)
        };
        let (initial, _) = make();
        let wire = drain(send(initial, limits()).await.unwrap().into_body()).await;
        let payload = decoded(&wire);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&payload).unwrap(),
            serde_json::json!({"error": {"code": "resource_exhausted",
                "message": if registration { "command registration capacity exceeded" }
                    else { "command registry capacity exceeded" }}})
        );
        let required = wire.len().max(payload.len() + 5);
        for budget in [required - 1, required] {
            let (source, slots) = make();
            let deadline = source.deadline;
            let result = send(
                source,
                reply::Limits {
                    wire_bytes: budget,
                    ..limits()
                },
            )
            .await;
            if budget == required {
                let mut body = result.unwrap().into_body();
                let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
                assert_eq!(decoded(&bytes), payload);
                let copy = bytes.clone();
                drop((bytes, body));
                assert_eq!(slots.available_permits(), 0);
                drop(copy);
            } else {
                let failure = result.unwrap_err();
                assert_eq!(failure.error, reply::Error::ResponseLimit);
                assert_eq!(failure.source.deadline, deadline);
                assert_eq!(slots.available_permits(), 0);
                drop(failure);
            }
            released(slots).await;
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn registration_capacity_requires_exact_pre_claim_fact_and_original_deadline() {
    use crate::command::RegistrationError;
    use sandproof_guest_runtime::pid_claims::Error as ClaimError;
    let fact = Rejection::RegistrationCapacity;
    assert!(fact.matches(&RequestError::Registration(RegistrationError::Capacity)));
    for error in [
        RequestError::Registration(RegistrationError::Storage(ClaimError::Busy)),
        RequestError::Registration(RegistrationError::Storage(ClaimError::WriteUnknown)),
        RequestError::Registration(RegistrationError::WorkerUnavailable),
        RequestError::DeadlineExceeded,
    ] {
        assert!(!fact.matches(&error));
    }
    let (mut source, slots) = source("unused", false);
    source.rejection = Some(Rejection::RegistrationCapacity);
    source.deadline = Instant::now();
    let failure = send(source, limits()).await.unwrap_err();
    assert_eq!(failure.error, reply::Error::DeadlineExceeded);
    assert_eq!(slots.available_permits(), 0);
    drop(failure);
    released(slots).await;
}
