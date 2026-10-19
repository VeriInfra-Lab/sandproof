use super::*;
use crate::file_rpc_response::{self, Limits as ResponseLimits};
use http_body_util::BodyExt;

const PAYLOAD: &[u8] = b"{\"code\":401,\"message\":\"unauthorized access, please provide a valid access token or method signing if supported\"}\n";
fn response_limits() -> ResponseLimits {
    ResponseLimits {
        wire_bytes: 1024,
        chunk_bytes: 17,
    }
}
async fn denial(session: &Session<Meta>, pool: &FileRpcRequests, tokens: &[Vec<u8>]) -> Failure {
    let mut headers = HeaderMap::new();
    for token in tokens {
        headers.append("x-access-token", HeaderValue::from_bytes(token).unwrap());
    }
    pool.admit()
        .unwrap()
        .authenticate(session, headers)
        .await
        .err()
        .expect("token must be denied")
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let chunk = frame.unwrap().into_data().unwrap();
        bytes.extend_from_slice(&chunk);
    }
    bytes
}

#[tokio::test]
async fn token_denials_keep_original_policy_private_and_use_outer_api_json() {
    let initialized = session().await;
    let uninitialized = Session::new(Meta, initialized.limits()).unwrap();
    let pool = FileRpcRequests::new(limits()).unwrap();
    for session in [&initialized, &uninitialized] {
        for values in [
            vec![],
            vec![vec![]],
            vec![b"private-secret".to_vec()],
            vec![vec![0x80]],
            vec![vec![b'x'; 65]],
            vec![b"token".to_vec(), b"token".to_vec()],
        ] {
            let failure = denial(session, &pool, &values).await;
            assert_eq!(
                failure.error(),
                &Error::Authorization(ServiceError::Unauthenticated)
            );
            assert!(!format!("{failure:?}").contains("private-secret"));
            assert!(failure.has_response());
            let response = failure
                .into_response(ResponseLimits {
                    wire_bytes: PAYLOAD.len(),
                    ..response_limits()
                })
                .await
                .unwrap();
            assert_eq!(response.status(), 401);
            assert_eq!(
                response.headers()["content-type"],
                "application/json; charset=utf-8"
            );
            assert_eq!(
                response.headers()["content-length"],
                PAYLOAD.len().to_string()
            );
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            assert!(!response.headers().contains_key("accept-encoding"));
            full(&pool);
            assert_eq!(drain(response.into_body()).await, PAYLOAD);
            released(&pool).await;
        }
    }
    // The original successful token path and opaque authorization remain intact.
    drop(
        pool.admit()
            .unwrap()
            .authenticate(&initialized, headers())
            .await
            .unwrap(),
    );
    released(&pool).await;
}

#[tokio::test]
async fn token_response_owns_last_bytes_and_checks_full_head_budget_and_original_expiry() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    for head in [false, true] {
        let failed = denial(&session, &pool, &[]).await;
        let deadline = failed.deadline();
        let failed = failed
            .into_authorization_response(
                ResponseLimits {
                    wire_bytes: PAYLOAD.len() - 1,
                    ..response_limits()
                },
                head,
            )
            .await
            .unwrap_err();
        assert_eq!(failed.error(), file_rpc_response::Error::ResponseLimit);
        assert_eq!(failed.deadline(), deadline);
        full(&pool);
        drop(failed);
        released(&pool).await;

        let failed = denial(&session, &pool, &[]).await;
        let deadline = failed.deadline();
        let response = failed
            .into_authorization_response(response_limits(), head)
            .await
            .unwrap();
        assert_eq!(
            response.headers()["content-length"],
            PAYLOAD.len().to_string()
        );
        // No completed auth read lock remains pinned by the response.
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        full(&pool);
        let mut body = response.into_body();
        if head {
            assert!(body.frame().await.is_none());
            drop(body);
        } else {
            let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
            let copy = first.clone();
            drop(first);
            tokio::time::pause();
            tokio::time::advance(deadline.saturating_duration_since(Instant::now())).await;
            assert!(body.frame().await.unwrap().is_err());
            tokio::time::resume();
            drop(body);
            full(&pool);
            drop(copy);
        }
        released(&pool).await;
    }
    let failed = denial(&session, &pool, &[]).await;
    let deadline = failed.deadline();
    tokio::time::pause();
    tokio::time::advance(deadline.saturating_duration_since(Instant::now())).await;
    let failed = failed.into_response(response_limits()).await.unwrap_err();
    assert_eq!(failed.error(), file_rpc_response::Error::DeadlineExceeded);
    assert_eq!(failed.deadline(), deadline);
    full(&pool);
    drop(failed);
    tokio::time::resume();
    released(&pool).await;

    let mut oversized = headers();
    oversized.insert(
        "ignored",
        HeaderValue::from_bytes(&vec![b'x'; 256]).unwrap(),
    );
    let failed = pool
        .admit()
        .unwrap()
        .authenticate(&session, oversized)
        .await
        .err()
        .unwrap();
    assert_eq!(failed.error(), &Error::HeaderLimit);
    assert!(!failed.has_response());
    let failed = failed
        .into_authorization_response(response_limits(), false)
        .await
        .unwrap_err();
    assert_eq!(failed.error(), file_rpc_response::Error::UnclassifiedFault);
    full(&pool);
    drop(failed);
    released(&pool).await;
}

#[tokio::test]
#[ignore = "explicit original WithAuthorization / Go HTTP; set SANDPROOF_GUEST_AUTH_REFERENCE"]
async fn original_outer_token_middleware_matches_complete_http_rejections() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_AUTH_REFERENCE");
    let mut vectors = vec![
        vec![],
        vec![vec![]],
        vec![b"wrong".to_vec()],
        vec![b"TOKEN".to_vec()],
        vec![vec![b'x'; 65]],
        vec![b"wrong".to_vec(), b"token".to_vec()],
    ];
    vectors.extend((0x80..=0xff).map(|byte| vec![vec![byte]]));
    let mut count = 0;
    for method in ["POST", "GET", "HEAD", "post"] {
        for values in &vectors {
            let failed = denial(&session, &pool, values).await;
            let response = failed
                .into_authorization_response(response_limits(), method == "HEAD")
                .await
                .unwrap();
            let mut observed = vec![response.status().as_u16().to_string()];
            for name in [
                "content-type",
                "content-length",
                "x-content-type-options",
                "accept-encoding",
                "content-encoding",
            ] {
                observed.push(hex(response
                    .headers()
                    .get(name)
                    .map_or(b"", |value| value.as_bytes())));
            }
            observed.push(hex(&drain(response.into_body()).await));
            let input = serde_json::json!({"Method":method, "Headers":values.iter().map(|v| hex(v)).collect::<Vec<_>>()});
            oracle.check(
                &format!("R\t{}", hex(&serde_json::to_vec(&input).unwrap())),
                &observed.join("\t"),
            );
            released(&pool).await;
            count += 1;
        }
    }
    oracle.finish();
    eprintln!(
        "{count} original outer token HTTP rejections; controlled initialized token store, no MMDS/Basic/files."
    );
}
