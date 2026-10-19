use super::*;
use crate::command_requests::http::Rpc;
use axum::http::Method;

#[tokio::test]
async fn oversized_arguments_preserve_accounts_and_request_ownership() {
    let session = session().await;
    let mut request_limits = limits();
    request_limits.body.total_bytes = 4096;
    request_limits.body.frame_bytes = 4096;
    let pool = CommandRequests::new(request_limits).unwrap();
    let fixture = StartFixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let accounts = AccountPool::new(Accounts(calls.clone()), 1, Duration::from_secs(30)).unwrap();
    let payload = serde_json::to_vec(&serde_json::json!({"process": {
        "cmd": "/must-not-run", "args": vec![""; 1000]
    }}))
    .unwrap();
    for gzip in [false, true] {
        for (default, basic, expected, lookups) in [
            (
                "alice",
                None,
                Error::Context(ContextError::ResourceExhausted),
                1,
            ),
            (
                "missing",
                None,
                Error::Context(ContextError::Unauthenticated),
                1,
            ),
            (
                "missing:invalid",
                None,
                Error::Context(ContextError::Unauthenticated),
                0,
            ),
            (
                "missing",
                Some("Basic YWxpY2U6"),
                Error::Context(ContextError::ResourceExhausted),
                1,
            ),
        ] {
            session
                .initialize(
                    &serde_json::to_vec(&serde_json::json!({
                        "accessToken": "token", "defaultUser": default
                    }))
                    .unwrap(),
                )
                .await
                .unwrap();
            let before = calls.load(Ordering::SeqCst);
            let mut headers = headers();
            headers.insert("content-type", "application/connect+json".parse().unwrap());
            if let Some(basic) = basic {
                headers.insert("authorization", basic.parse().unwrap());
            }
            if gzip {
                headers.insert("connect-content-encoding", "gzip".parse().unwrap());
            }
            let selected = pool
                .admit()
                .unwrap()
                .authenticate_http(&session, &Method::POST, headers)
                .await
                .unwrap()
                .prepare_http(&Method::POST, &accounts, context_limits())
                .await
                .unwrap()
                .select_json(&Method::POST, Rpc::Start)
                .unwrap();
            let body = selected_body(&payload, gzip);
            let ready = selected.decode(body).await.unwrap();
            assert_eq!(
                ready.request(),
                &Request::StartArgumentsExceeded { limit: 32 }
            );
            assert_eq!(
                calls.load(Ordering::SeqCst) - before,
                usize::from(basic.is_some())
            );
            let failure = ready
                .start_pid(
                    &accounts,
                    &settings(),
                    fixture.preparation(),
                    response_limits(),
                )
                .await
                .err()
                .unwrap();
            assert_eq!(failure.error(), expected);
            assert_eq!(calls.load(Ordering::SeqCst) - before, lookups);
            assert_eq!(fixture.registry.available_slots(), 1);
            drop(fixture.owners.reserve().unwrap());
            full(&pool);
            drop(failure);
            released(&pool).await;
        }
    }
}

fn selected_body(payload: &[u8], gzip: bool) -> Body {
    if !gzip {
        return framed(payload);
    }
    let compressed = crate::file_rpc_requests::tests::gzip::compressed(payload);
    let mut wire = vec![1];
    wire.extend(u32::try_from(compressed.len()).unwrap().to_be_bytes());
    wire.extend(compressed);
    Body::from(wire)
}
