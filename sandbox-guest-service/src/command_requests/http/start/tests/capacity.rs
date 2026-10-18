use super::*;

#[tokio::test]
async fn registry_capacity_reply_obeys_identity_gzip_bounds_and_original_owner() {
    for gzip in [false, true] {
        let make = || {
            let (mut source, slots) = source("unused", gzip);
            source.rejection = Some(Rejection::RegistryCapacity);
            (source, slots)
        };
        let (initial, _) = make();
        let wire = drain(send(initial, limits()).await.unwrap().into_body()).await;
        let payload = decoded(&wire);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&payload).unwrap(),
            serde_json::json!({"error": {"code": "resource_exhausted",
                "message": "command registry capacity exceeded"}})
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
