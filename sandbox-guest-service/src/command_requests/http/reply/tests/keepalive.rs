use super::*;

#[tokio::test]
async fn approved_keepalive_error_uses_bounded_negotiated_eos_and_retains_original_capacity() {
    for rpc in [Rpc::Start, Rpc::Connect] {
        for gzip in [false, true] {
            let (_, format) = rejected(rpc, gzip);
            let error = SelectionError::Keepalive(crate::command_keepalive::InvalidInterval);
            let at = deadline();
            let (owner, slots) = retain();
            let response = send(
                error.clone(),
                format,
                Limits {
                    chunk_bytes: 16384,
                    ..limits()
                },
                at,
                owner,
            )
            .await
            .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()["content-type"],
                "application/connect+json; charset=utf-8"
            );
            assert_eq!(
                response.headers().contains_key("connect-content-encoding"),
                gzip
            );
            let mut body = response.into_body();
            let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
            assert_eq!(bytes[0], if gzip { 3 } else { 2 });
            assert_eq!(
                u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize,
                bytes.len() - 5
            );
            let raw = if gzip {
                decoded(&bytes[5..])
            } else {
                bytes[5..].to_vec()
            };
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&raw).unwrap(),
                serde_json::json!({"error":{
                    "code":"invalid_argument", "message":"keepalive interval must be between 1 and 9223372036 seconds"
                }})
            );
            let copy = bytes.clone();
            drop(body);
            drop(bytes);
            assert_eq!(slots.available_permits(), 0);
            drop(copy);
            released(slots).await;
            let (owner, slots) = retain();
            let failure = send(
                error.clone(),
                format,
                Limits {
                    wire_bytes: 5 + raw.len() - 1,
                    ..limits()
                },
                at,
                owner,
            )
            .await
            .unwrap_err();
            assert_eq!(failure.error(), Error::ResponseLimit);
            assert_eq!(failure.selection_error(), &error);
            assert_eq!(failure.deadline(), at);
            assert_eq!(slots.available_permits(), 0);
            drop(failure);
            released(slots).await;
        }
    }
    let (_, unary) = rejected(Rpc::List, false);
    let failure = send(
        SelectionError::Keepalive(crate::command_keepalive::InvalidInterval),
        unary,
        limits(),
        deadline(),
        Arc::new(()),
    )
    .await
    .unwrap_err();
    assert_eq!(failure.error(), Error::UnclassifiedFault);
}
