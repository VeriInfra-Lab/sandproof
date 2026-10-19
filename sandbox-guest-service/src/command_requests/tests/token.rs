use super::*;
use crate::{
    command_requests::http::{authorization, reply},
    file_download::tests::reference::{Reference, hex},
};
use axum::http::{HeaderValue, Method};
use http_body_util::BodyExt;

const PAYLOAD: &[u8] = b"{\"code\":401,\"message\":\"unauthorized access, please provide a valid access token or method signing if supported\"}\n";
fn response_limits() -> reply::Limits {
    reply::Limits {
        wire_bytes: PAYLOAD.len(),
        chunk_bytes: 17,
    }
}
async fn denial(
    session: &Session<Meta>,
    pool: &CommandRequests,
    method: &Method,
    tokens: &[Vec<u8>],
) -> authorization::Failure {
    let mut headers = HeaderMap::new();
    for token in tokens {
        headers.append("x-access-token", HeaderValue::from_bytes(token).unwrap());
    }
    pool.admit()
        .unwrap()
        .authenticate_http(session, method, headers)
        .await
        .err()
        .expect("token must be denied")
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        bytes.extend_from_slice(&frame.unwrap().into_data().unwrap());
    }
    bytes
}

#[tokio::test]
async fn token_denial_precedes_basic_method_and_protocol_with_unchanged_local_policy() {
    let initialized = session().await;
    let uninitialized = Session::new(Meta, initialized.limits()).unwrap();
    let mut settings = limits();
    settings.headers.bytes = 2048;
    settings.headers.fields = 16;
    let pool = CommandRequests::new(settings).unwrap();
    for session in [&initialized, &uninitialized] {
        for tokens in [
            vec![],
            vec![vec![]],
            vec![b"private-secret".to_vec()],
            vec![vec![0x80]],
            vec![vec![b'x'; 65]],
            vec![b"token".to_vec(), b"token".to_vec()],
        ] {
            for method in [Method::POST, Method::GET, Method::HEAD] {
                let mut headers = HeaderMap::new();
                for value in &tokens {
                    headers.append("x-access-token", HeaderValue::from_bytes(value).unwrap());
                }
                for (name, value) in [
                    ("authorization", "Basic Og=="),
                    ("content-type", "application/connect+json"),
                    ("connect-protocol-version", "wrong"),
                    ("connect-content-encoding", "wrong"),
                    ("connect-accept-encoding", "gzip"),
                    ("connect-timeout-ms", "0"),
                ] {
                    headers.insert(name, value.parse().unwrap());
                }
                let admitted = pool.admit().unwrap();
                let deadline = admitted.deadline();
                let failure = admitted
                    .authenticate_http(session, &method, headers)
                    .await
                    .err()
                    .unwrap();
                assert_eq!(
                    failure.error(),
                    Error::Authorization(ServiceError::Unauthenticated)
                );
                assert_eq!(failure.deadline(), deadline);
                assert!(!format!("{failure:?}").contains("private-secret"));
                let response = failure.into_response(response_limits()).await.unwrap();
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
                for name in [
                    "accept-encoding",
                    "content-encoding",
                    "connect-content-encoding",
                    "connect-accept-encoding",
                    "allow",
                ] {
                    assert!(!response.headers().contains_key(name));
                }
                full(&pool);
                assert_eq!(
                    drain(response.into_body()).await,
                    if method == Method::HEAD { b"" } else { PAYLOAD }
                );
                released(&pool).await;
            }
        }
    }
    let authenticated = pool
        .admit()
        .unwrap()
        .authenticate_http(&initialized, &Method::POST, headers())
        .await
        .unwrap();
    assert_eq!(authenticated.headers()["x-access-token"], "token");
    full(&pool);
    drop(authenticated);
    released(&pool).await;
}

#[tokio::test]
async fn token_response_retains_original_deadline_last_bytes_and_head_budget() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    for method in [Method::POST, Method::HEAD] {
        for wire_bytes in [0, PAYLOAD.len() - 1] {
            let failed = denial(&session, &pool, &method, &[]).await;
            let deadline = failed.deadline();
            let failed = failed
                .into_response(reply::Limits {
                    wire_bytes,
                    ..response_limits()
                })
                .await
                .unwrap_err();
            assert_eq!(
                failed.error(),
                if wire_bytes == 0 {
                    reply::Error::InvalidConfiguration
                } else {
                    reply::Error::ResponseLimit
                }
            );
            assert_eq!(
                failed.request_error(),
                Error::Authorization(ServiceError::Unauthenticated)
            );
            assert_eq!(failed.deadline(), deadline);
            full(&pool);
            drop(failed);
            released(&pool).await;
        }
        let failed = denial(&session, &pool, &method, &[]).await;
        let deadline = failed.deadline();
        let response = failed.into_response(response_limits()).await.unwrap();
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        full(&pool);
        let mut body = response.into_body();
        if method == Method::HEAD {
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
    let failure = denial(&session, &pool, &Method::POST, &[]).await;
    let deadline = failure.deadline();
    tokio::time::pause();
    tokio::time::advance(deadline.saturating_duration_since(Instant::now())).await;
    let failed = failure.into_response(response_limits()).await.unwrap_err();
    assert_eq!(failed.error(), reply::Error::DeadlineExceeded);
    assert_eq!(failed.deadline(), deadline);
    full(&pool);
    drop(failed);
    tokio::time::resume();
    released(&pool).await;
}

#[tokio::test]
async fn non_token_failures_never_fabricate_a_token_denial() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    for expired in [false, true] {
        let admitted = pool.admit().unwrap();
        let deadline = admitted.deadline();
        let mut headers = headers();
        if expired {
            tokio::time::pause();
            tokio::time::advance(deadline.saturating_duration_since(Instant::now())).await;
        } else {
            headers.insert("private", "private-secret".repeat(30).parse().unwrap());
        }
        let failed = admitted
            .authenticate_http(&session, &Method::POST, headers)
            .await
            .err()
            .unwrap();
        let error = if expired {
            Error::DeadlineExceeded
        } else {
            Error::HeaderLimit
        };
        assert_eq!(failed.error(), error);
        let failed = failed.into_response(response_limits()).await.unwrap_err();
        assert_eq!(failed.error(), reply::Error::UnclassifiedFault);
        assert_eq!(failed.request_error(), error);
        assert_eq!(failed.deadline(), deadline);
        assert!(!format!("{failed:?}").contains("private-secret"));
        full(&pool);
        drop(failed);
        if expired {
            tokio::time::resume();
        }
        released(&pool).await;
    }
}

#[tokio::test]
#[ignore = "explicit original WithAuthorization / Go HTTP; set SANDPROOF_GUEST_AUTH_REFERENCE"]
async fn command_token_replies_match_original_outer_http_bytes() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
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
    let paths = [
        "Start",
        "Connect",
        "List",
        "SendInput",
        "CloseStdin",
        "SendSignal",
        "Update",
    ];
    for method in [
        Method::POST,
        Method::GET,
        Method::HEAD,
        Method::from_bytes(b"post").unwrap(),
    ] {
        for values in &vectors {
            let failed = denial(&session, &pool, &method, values).await;
            let response = failed.into_response(response_limits()).await.unwrap();
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
                    .map_or(b"", HeaderValue::as_bytes)));
            }
            observed.push(hex(&drain(response.into_body()).await));
            let path = format!("/process.Process/{}", paths[count % paths.len()]);
            let input = serde_json::json!({"Method":method.as_str(), "Path":path, "Headers":values.iter().map(|v| hex(v)).collect::<Vec<_>>()});
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
        "{count} original outer token HTTP rejections through command admission; controlled initialized store, no Basic/commands."
    );
}
