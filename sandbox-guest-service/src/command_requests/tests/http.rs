use super::*;
use crate::command_requests::http::{Error as SelectionError, Rpc};
use axum::http::Method;

fn unread() -> Body {
    Body::from_stream(stream::poll_fn(
        |_| -> Poll<Option<Result<Bytes, std::io::Error>>> {
            panic!("rejected HTTP selection polled body")
        },
    ))
}
async fn prepare_headers(
    pool: &CommandRequests,
    session: &Session<Meta>,
    h: HeaderMap,
) -> Prepared {
    let accounts = AccountPool::new(
        Accounts(Arc::new(AtomicUsize::new(0))),
        1,
        Duration::from_secs(5),
    )
    .unwrap();
    pool.admit()
        .unwrap()
        .authenticate(session, h)
        .await
        .unwrap()
        .prepare(&accounts, context_limits())
        .await
        .unwrap()
}

#[tokio::test]
async fn http_selection_failures_keep_original_capacity_without_holding_completed_auth() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    for case in 0..6 {
        let mut h = headers();
        h.insert("content-type", "application/connect+json".parse().unwrap());
        let method = if case == 0 { Method::GET } else { Method::POST };
        match case {
            1 => {
                h.insert("content-type", "invalid".parse().unwrap());
            }
            2 => {
                h.insert("connect-content-encoding", "br".parse().unwrap());
            }
            3 => {
                h.insert("connect-protocol-version", "bad".parse().unwrap());
            }
            4 => {
                h.insert("connect-timeout-ms", "bad".parse().unwrap());
            }
            _ => {}
        }
        let prepared = prepare_headers(&pool, &session, h).await;
        let deadline = prepared.deadline();
        if case == 5 {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(31)).await;
            tokio::time::resume();
        }
        let failure = prepared.select_json(&method, Rpc::Start).err().unwrap();
        assert_eq!(failure.deadline(), deadline);
        assert!(matches!(
            (case, failure.error()),
            (0, SelectionError::Method)
                | (1, SelectionError::ContentType)
                | (2, SelectionError::Compression(_))
                | (3, SelectionError::Version(_))
                | (4, SelectionError::Timeout(_))
                | (5, SelectionError::Request(Error::DeadlineExceeded))
        ));
        full(&pool);
        // A response/failure retains capacity, not a completed auth read lock.
        tokio::time::timeout(
            Duration::from_secs(1),
            session.initialize(br#"{"accessToken":"token"}"#),
        )
        .await
        .unwrap()
        .unwrap();
        drop(failure);
        released(&pool).await;
    }
}

#[tokio::test]
async fn identity_decoder_rejects_expired_or_gzip_selection_before_reading_and_keeps_ready_ownership()
 {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    for field in [
        "connect-content-encoding",
        "connect-accept-encoding",
        "connect-timeout-ms",
    ] {
        let mut h = headers();
        h.insert("content-type", "application/connect+json".parse().unwrap());
        h.insert(
            field,
            if field == "connect-timeout-ms" {
                "0"
            } else {
                "gzip"
            }
            .parse()
            .unwrap(),
        );
        let selected = prepare_headers(&pool, &session, h)
            .await
            .select_json(&Method::POST, Rpc::Start)
            .unwrap();
        let deadline = selected.deadline();
        let failure = selected.decode_identity(unread()).await.err().unwrap();
        assert_eq!(failure.deadline(), deadline);
        assert_eq!(
            failure.error(),
            &if field == "connect-timeout-ms" {
                SelectionError::Request(Error::DeadlineExceeded)
            } else {
                SelectionError::EncodingNotImplemented
            }
        );
        full(&pool);
        drop(failure);
        released(&pool).await;
    }
    let mut h = headers();
    h.insert(
        "content-type",
        "APPLICATION/JSON; charset=\"UTF-8\"".parse().unwrap(),
    );
    h.insert("content-encoding", "identity".parse().unwrap());
    h.insert("connect-timeout-ms", "1000".parse().unwrap());
    let selected = prepare_headers(&pool, &session, h)
        .await
        .select_json(&Method::POST, Rpc::List)
        .unwrap();
    assert_eq!(selected.content_type(), "application/json; charset=utf-8");
    let deadline = selected.deadline();
    let ready = selected.decode_identity(Body::from("{}")).await.unwrap();
    assert_eq!(ready.deadline(), deadline);
    assert_eq!(ready.request(), &Request::List);
    full(&pool);
    drop(ready);
    released(&pool).await;
}
