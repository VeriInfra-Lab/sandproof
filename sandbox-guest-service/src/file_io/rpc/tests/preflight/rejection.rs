use super::*;
use crate::file_rpc_response::{self, Rejection};
use http_body_util::BodyExt;

async fn missing(
    pool: &FilePool,
    requests: &FileRpcRequests,
    session: &Session<Meta>,
    root: &Path,
    default: bool,
) -> RpcFailure {
    let accounts = AccountPool::new(Missing, 1, Duration::from_secs(5)).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-access-token", "token".parse().unwrap());
    if !default {
        headers.insert("authorization", "Basic dXNlcjo=".parse().unwrap());
        headers.insert("connect-protocol-version", "2".parse().unwrap());
        headers.insert("connect-timeout-ms", "invalid".parse().unwrap());
    }
    let auth = requests
        .admit()
        .unwrap()
        .authenticate(session, headers)
        .await
        .unwrap();
    let prepared = pool.prepare_rpc(auth, &accounts, settings(root)).await;
    if default {
        prepared
            .unwrap()
            .decode("MakeDir", Body::from(r#"{"path":"auth-refused"}"#))
            .await
            .unwrap()
            .execute(&accounts)
            .await
            .unwrap_err()
    } else {
        // No decoder/body is reached, even with invalid version and timeout.
        prepared.err().unwrap()
    }
}

#[tokio::test]
async fn header_rejection_precedes_default_lookup_and_keeps_both_slots_to_last_byte_copy() {
    for explicit in [false, true] {
        for version in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let session = session().await;
            let pool = crate::file_io::tests::pool();
            let requests = requests();
            let (accounts, calls) = accounts(root.path(), None);
            let mut headers = HeaderMap::new();
            headers.insert("x-access-token", "token".parse().unwrap());
            if explicit {
                headers.insert("authorization", "Basic dXNlcjo=".parse().unwrap());
            }
            if version {
                headers.insert("connect-protocol-version", "2".parse().unwrap());
            }
            headers.insert("connect-timeout-ms", "invalid".parse().unwrap());
            let auth = requests
                .admit()
                .unwrap()
                .authenticate(&session, headers)
                .await
                .unwrap();
            let prepared = pool
                .prepare_rpc(auth, &accounts, settings(root.path()))
                .await
                .unwrap();
            let end = prepared.deadline();
            let failure = prepared
                .decode("MakeDir", Body::from(r#"{"path":"refused"}"#))
                .await
                .err()
                .unwrap();
            assert_eq!(
                matches!(failure.error(), file_rpc_requests::Error::Version(_)),
                version
            );
            assert_eq!(
                matches!(failure.error(), file_rpc_requests::Error::Timeout(_)),
                !version
            );
            assert_eq!(failure.deadline(), end);
            assert_eq!(calls.load(Ordering::SeqCst), usize::from(explicit));
            session
                .initialize(br#"{"accessToken":"token"}"#)
                .await
                .unwrap();
            full(&pool, &requests);
            let response = failure.into_response(response_limits()).await.unwrap();
            assert_eq!(response.status(), 400);
            let mut body = response.into_body();
            let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
            let copy = bytes.clone();
            drop(bytes);
            drop(body);
            full(&pool, &requests);
            assert!(!root.path().join("refused").exists());
            drop(copy);
            free(&pool, &requests).await;
        }
    }
}
fn response_limits() -> file_rpc_response::Limits {
    file_rpc_response::Limits {
        wire_bytes: 4096,
        chunk_bytes: 7,
    }
}

#[tokio::test]
async fn missing_explicit_and_default_accounts_keep_their_actual_origin_and_fixed_text() {
    for default in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let session = session().await;
        let pool = crate::file_io::tests::pool();
        let requests = requests();
        let failure = missing(&pool, &requests, &session, root.path(), default).await;
        assert_eq!(
            failure.error(),
            RpcError::Context(ContextError::Unauthenticated)
        );
        assert_eq!(
            matches!(failure.rejection(), Some(Rejection::DefaultUser(_))),
            default
        );
        assert_eq!(
            matches!(failure.rejection(), Some(Rejection::ExplicitUser(_))),
            !default
        );
        full(&pool, &requests);
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        let response = failure.into_response(response_limits()).await.unwrap();
        assert_eq!(response.status(), 401);
        assert_eq!(
            response
                .headers()
                .get("accept-encoding")
                .map(|v| v.to_str().unwrap()),
            default.then_some("gzip")
        );
        let mut body = response.into_body();
        let mut data = Vec::new();
        while let Some(frame) = body.frame().await {
            let bytes = frame.unwrap().into_data().unwrap();
            data.extend_from_slice(&bytes);
        }
        let value: serde_json::Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(value["code"], "unauthenticated");
        assert_eq!(
            value["message"],
            if default {
                "invalid default user: 'user'"
            } else {
                "invalid username: 'user'"
            }
        );
        free(&pool, &requests).await;
        assert!(!root.path().join("auth-refused").exists());
    }
}

#[tokio::test]
async fn rpc_rejection_body_copy_keeps_original_request_and_file_capacity() {
    for default in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let session = session().await;
        let pool = crate::file_io::tests::pool();
        let requests = requests();
        let failure = missing(&pool, &requests, &session, root.path(), default).await;
        let response = failure.into_response(response_limits()).await.unwrap();
        let mut body = response.into_body();
        let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
        let copy = bytes.clone();
        drop(bytes);
        drop(body);
        full(&pool, &requests);
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        drop(copy);
        free(&pool, &requests).await;
    }
}

#[tokio::test]
async fn rpc_rejection_encoding_failure_and_expiry_keep_both_slots_and_original_deadline() {
    for expired in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let session = session().await;
        let pool = crate::file_io::tests::pool();
        let requests = requests();
        let failure = missing(&pool, &requests, &session, root.path(), false).await;
        let end = failure.deadline();
        let mut limits = response_limits();
        if expired {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(6)).await;
        } else {
            limits.wire_bytes = 1;
        }
        let failed = failure.into_response(limits).await.unwrap_err();
        assert_eq!(
            failed.error(),
            if expired {
                file_rpc_response::Error::DeadlineExceeded
            } else {
                file_rpc_response::Error::ResponseLimit
            }
        );
        assert_eq!(failed.deadline(), end);
        if expired {
            tokio::time::resume();
        }
        full(&pool, &requests);
        drop(failed);
        free(&pool, &requests).await;
    }
}

struct Unavailable;
impl AccountSource for Unavailable {
    fn lookup(&self, _: &str) -> Result<Account, AccountError> {
        Err(AccountError::Unavailable)
    }
}

#[tokio::test]
async fn source_fault_and_duplicate_basic_are_not_invented_missing_user_responses() {
    for duplicate in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let session = session().await;
        let pool = crate::file_io::tests::pool();
        let requests = requests();
        let accounts = AccountPool::new(Unavailable, 1, Duration::from_secs(5)).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-access-token", "token".parse().unwrap());
        headers.insert("authorization", "Basic dXNlcjpzZWNyZXQ=".parse().unwrap());
        if duplicate {
            headers.append("authorization", "Basic Og==".parse().unwrap());
        }
        let auth = requests
            .admit()
            .unwrap()
            .authenticate(&session, headers)
            .await
            .unwrap();
        let failure = pool
            .prepare_rpc(auth, &accounts, settings(root.path()))
            .await
            .err()
            .unwrap();
        assert_eq!(
            failure.error(),
            RpcError::Context(if duplicate {
                ContextError::Unauthenticated
            } else {
                ContextError::Unavailable
            })
        );
        assert!(failure.rejection().is_none());
        let failed = failure.into_response(response_limits()).await.unwrap_err();
        assert_eq!(failed.error(), file_rpc_response::Error::UnclassifiedFault);
        assert!(!format!("{failed:?}").contains("secret"));
        full(&pool, &requests);
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        drop(failed);
        free(&pool, &requests).await;
    }
}
