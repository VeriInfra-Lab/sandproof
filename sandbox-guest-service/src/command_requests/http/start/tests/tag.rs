use super::*;

#[tokio::test]
async fn duplicate_tag_reply_uses_approved_code_exact_text_and_original_capacity() {
    for tag in [
        "",
        "123",
        "a'b\"\\<>&中\u{2028}",
        "\0\n\r",
        &"x".repeat(512),
    ] {
        for gzip in [false, true] {
            let (mut original, slots) = source(tag, gzip);
            original.rejection = Some(Rejection::TagAlreadyExists(tag.into()));
            let response = send(original, limits()).await.unwrap();
            assert_eq!(response.status(), 200);
            let mut body = response.into_body();
            let data = body.frame().await.unwrap().unwrap().into_data().unwrap();
            let last = data.clone();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&decoded(&data)).unwrap(),
                serde_json::json!({"error":{"code":"already_exists",
                    "message":format!("process with tag {tag} already exists")}})
            );
            drop((data, body));
            assert_eq!(slots.available_permits(), 0);
            drop(last);
            released(slots).await;
        }
    }
}

#[tokio::test]
async fn duplicate_tag_encoding_obeys_exact_wire_limit_and_keeps_failure_responsibility() {
    for gzip in [false, true] {
        let tag = "long\\<>&中".repeat(50);
        let (mut original, _) = source(&tag, gzip);
        original.rejection = Some(Rejection::TagAlreadyExists(tag.clone()));
        let bytes = drain(send(original, limits()).await.unwrap().into_body()).await;
        let required = bytes.len().max(decoded(&bytes).len() + 5);
        for budget in [required - 1, required] {
            let (mut original, slots) = source(&tag, gzip);
            original.rejection = Some(Rejection::TagAlreadyExists(tag.clone()));
            let deadline = original.deadline;
            let result = send(
                original,
                reply::Limits {
                    wire_bytes: budget,
                    ..limits()
                },
            )
            .await;
            if budget == required {
                assert_eq!(
                    decoded(&drain(result.unwrap().into_body()).await),
                    decoded(&bytes)
                );
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
#[test]
fn duplicate_tag_fact_requires_original_reservation_conflict() {
    use crate::command::RegistryError;
    let fact = Rejection::TagAlreadyExists("original".into());
    assert!(fact.matches(&RequestError::Registry(RegistryError::AlreadyExists)));
    for error in [
        RequestError::Registry(RegistryError::NotFound),
        RequestError::Registry(RegistryError::ResourceExhausted),
        RequestError::Registry(RegistryError::Unavailable),
        RequestError::DeadlineExceeded,
    ] {
        assert!(!fact.matches(&error));
    }
}
