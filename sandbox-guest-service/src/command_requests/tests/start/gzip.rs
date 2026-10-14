use super::*;
use crate::command_requests::http::{Error as SelectionError, Rpc};
use crate::file_rpc_requests::tests::gzip::compressed;
use axum::http::Method;
use sandproof_guest_runtime::protocol::FramingError;

fn wire(flag: u8, payload: &[u8]) -> Vec<u8> {
    let mut result = vec![flag];
    result.extend(u32::try_from(payload.len()).unwrap().to_be_bytes());
    result.extend(payload);
    result
}
fn chunks(bytes: &[u8]) -> Body {
    Body::from_stream(stream::iter(
        bytes
            .chunks(17)
            .map(|b| Ok::<_, std::io::Error>(Bytes::copy_from_slice(b)))
            .collect::<Vec<_>>(),
    ))
}

fn gzip_headers() -> HeaderMap {
    let mut h = headers();
    h.insert(
        "content-type",
        "application/connect+json; charset=utf-8".parse().unwrap(),
    );
    h.insert("connect-content-encoding", "gzip".parse().unwrap());
    h
}

#[tokio::test]
async fn gzip_framing_and_budgets_fail_before_default_account_or_start_preparation() {
    let session = session().await;
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"missing"}"#)
        .await
        .unwrap();
    let pool = CommandRequests::new(limits()).unwrap();
    let fixture = StartFixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let accounts = AccountPool::new(Accounts(calls.clone()), 1, Duration::from_secs(30)).unwrap();
    let expanded = compressed(format!("{{\"ignored\":\"{}\"}}", "x".repeat(1024)).as_bytes());
    for (payload, expected) in [
        (wire(1, b"not gzip"), Some("header")),
        (wire(1, &expanded), Some("limit")),
        (
            [
                wire(1, &compressed(b"{}")),
                wire(3, &compressed(b"{}")),
                vec![0],
            ]
            .concat(),
            Some("trailing"),
        ),
        (wire(1, b""), None),
        (
            [wire(1, &compressed(b"{}")), wire(3, &compressed(b"{}"))].concat(),
            None,
        ),
    ] {
        let h = gzip_headers();
        let selected = pool
            .admit()
            .unwrap()
            .authenticate_http(&session, &Method::POST, h)
            .await
            .unwrap()
            .prepare_http(&Method::POST, &accounts, context_limits())
            .await
            .unwrap()
            .select_json(&Method::POST, Rpc::Start)
            .unwrap();
        let deadline = selected.deadline();
        let before = calls.load(Ordering::SeqCst);
        let result = selected.decode(chunks(&payload)).await;
        assert_eq!(calls.load(Ordering::SeqCst), before);
        if let Some(kind) = expected {
            let failure = result.err().unwrap();
            assert!(matches!(
                (kind, failure.error()),
                ("header", SelectionError::Request(Error::Compression { .. }))
                    | ("limit", SelectionError::Request(Error::BodyLimit))
                    | (
                        "trailing",
                        SelectionError::Request(Error::Framing(FramingError::ExtraBytes(1)))
                    )
            ));
            assert_eq!(failure.deadline(), deadline);
            session
                .initialize(br#"{"accessToken":"token","defaultUser":"missing"}"#)
                .await
                .unwrap();
            full(&pool);
            drop(failure);
        } else {
            let failure = result
                .unwrap()
                .start_pid(
                    &accounts,
                    &settings(),
                    fixture.preparation(),
                    response_limits(),
                )
                .await
                .err()
                .unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), before + 1);
            let response = failure
                .into_default_account_response(crate::command_requests::http::reply::Limits {
                    wire_bytes: 4096,
                    chunk_bytes: 4096,
                })
                .await
                .unwrap();
            assert_eq!(
                response.headers()["content-type"],
                "application/connect+json; charset=utf-8"
            );
            assert_eq!(response.headers()["connect-content-encoding"], "gzip");
            let mut body = response.into_body();
            let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
            assert_eq!(bytes[0], 3);
            let copy = bytes.clone();
            drop(bytes);
            drop(body);
            full(&pool);
            drop(copy);
        }
        assert_eq!(fixture.registry.available_slots(), 1);
        drop(fixture.owners.reserve().unwrap());
        released(&pool).await;
    }
}
