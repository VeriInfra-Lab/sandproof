use super::*;
use crate::command_requests::http::account;
use axum::http::Method;

fn reply_limits() -> account::Limits {
    account::Limits {
        header_bytes: 1024,
        body: crate::command_requests::http::reply::Limits {
            wire_bytes: 1024,
            chunk_bytes: 1024,
        },
    }
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(frame) = body.frame().await {
        out.extend_from_slice(&frame.unwrap().into_data().unwrap());
    }
    out
}
#[tokio::test]
async fn original_explicit_account_rejection_precedes_bad_headers_and_keeps_response_ownership() {
    let session = session().await;
    let pool = CommandRequests::new(Limits {
        headers: HeaderLimits {
            bytes: 1024,
            fields: 16,
        },
        ..limits()
    })
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let accounts = AccountPool::new(Accounts(calls.clone()), 1, Duration::from_secs(30)).unwrap();
    for case in 0..5 {
        let method = match case {
            2 => Method::HEAD,
            3 => Method::GET,
            _ => Method::POST,
        };
        let mut h = headers();
        h.insert(
            "content-type",
            "APPLICATION/CONNECT+JSON;extra=x".parse().unwrap(),
        );
        h.insert(
            "connect-protocol-version",
            if case == 1 { "bad" } else { "1" }.parse().unwrap(),
        );
        h.insert("connect-content-encoding", "br".parse().unwrap());
        h.insert("connect-accept-encoding", "gzip".parse().unwrap());
        h.insert("connect-timeout-ms", "bad".parse().unwrap());
        h.insert(
            "authorization",
            if case == 4 {
                "Basic OnNlY3JldA=="
            } else {
                "Basic bWlzc2luZzpzZWNyZXQ="
            }
            .parse()
            .unwrap(),
        );
        let auth = pool
            .admit()
            .unwrap()
            .authenticate(&session, h)
            .await
            .unwrap();
        let original = auth.deadline();
        let failure = auth
            .prepare_http(&method, &accounts, context_limits())
            .await
            .err()
            .unwrap();
        assert_eq!(
            failure.error(),
            Error::Context(ContextError::Unauthenticated)
        );
        assert_eq!(failure.deadline(), original);
        assert!(!format!("{failure:?}").contains("missing"));
        full(&pool);
        tokio::time::timeout(
            Duration::from_secs(1),
            session.initialize(br#"{"accessToken":"token"}"#),
        )
        .await
        .unwrap()
        .unwrap();
        let response = failure.into_response(reply_limits()).await.unwrap();
        let stream = matches!(case, 0 | 4);
        assert_eq!(response.status(), if stream { 200 } else { 401 });
        let content = response.headers()["content-type"].clone();
        assert_eq!(
            content,
            if stream {
                "application/connect+json; extra=x"
            } else {
                "application/json"
            }
        );
        let bytes = drain(response.into_body()).await;
        if method == Method::HEAD {
            assert!(bytes.is_empty());
        } else {
            let value: serde_json::Value =
                serde_json::from_slice(if stream { &bytes[5..] } else { &bytes }).unwrap();
            let error = if stream { &value["error"] } else { &value };
            assert_eq!(
                error["message"],
                if case == 4 {
                    "invalid username: ''"
                } else {
                    "invalid username: 'missing'"
                }
            );
        }
        if stream {
            full(&pool);
        }
        drop(content);
        released(&pool).await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}

struct Fault(AccountError);
impl AccountSource for Fault {
    fn lookup(&self, _: &str) -> Result<Account, AccountError> {
        Err(self.0)
    }
}
#[tokio::test]
async fn account_source_faults_never_become_fabricated_missing_user_responses() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    for error in [
        AccountError::Unavailable,
        AccountError::ResourceExhausted,
        AccountError::InvalidArgument,
    ] {
        let accounts = AccountPool::new(Fault(error), 1, Duration::from_secs(30)).unwrap();
        let mut h = headers();
        h.insert("content-type", "application/json".parse().unwrap());
        h.insert("authorization", "Basic bWlzc2luZzo=".parse().unwrap());
        let failure = pool
            .admit()
            .unwrap()
            .authenticate(&session, h)
            .await
            .unwrap()
            .prepare_http(&Method::POST, &accounts, context_limits())
            .await
            .err()
            .unwrap();
        let original = failure.error();
        let deadline = failure.deadline();
        let reply = failure.into_response(reply_limits()).await.unwrap_err();
        assert_eq!(reply.error(), account::Error::UnclassifiedFault);
        assert_eq!(reply.request_error(), original);
        assert_eq!(reply.deadline(), deadline);
        full(&pool);
        drop(reply);
        released(&pool).await;
    }
}
