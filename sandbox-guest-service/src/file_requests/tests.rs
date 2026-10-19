use super::*;
use crate::{InitLimits, Metadata, ServiceError};
use axum::http::StatusCode;
use http_body_util::BodyExt;
use sha2::{Digest, Sha512};

struct Meta;
impl MetadataSource for Meta {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        Ok(Metadata {
            instance_id: "vm".into(),
            template_id: "tpl".into(),
            access_token_sha512: Sha512::digest(b"token").into(),
        })
    }
}
async fn session() -> Session<Meta> {
    let session = Session::new(
        Meta,
        InitLimits {
            body_bytes: 1024,
            token_bytes: 64,
            state_bytes: 1024,
            env_entries: 16,
            in_flight: 1,
            timeout: Duration::from_secs(5),
        },
    )
    .unwrap();
    session
        .initialize(br#"{"accessToken":"token"}"#)
        .await
        .unwrap();
    session
}
pub(crate) fn limits() -> Limits {
    Limits {
        in_flight: 1,
        timeout: Duration::from_secs(30),
        query: file_query::QueryLimits {
            raw_bytes: 1024,
            fields: 16,
        },
        authorization: auth::Limits {
            headers: file_upload::HeaderLimits {
                bytes: 1024,
                fields: 16,
            },
            query_bytes: 1024,
        },
        rejection: upload_response::Limits {
            wire_bytes: 4096,
            chunk_bytes: 4096,
        },
    }
}
fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-access-token", "token".parse().unwrap());
    headers
}
pub(crate) fn full(pool: &FileRequests) {
    assert!(matches!(
        pool.admit(),
        Err(AdmissionError::ResourceExhausted)
    ));
}
pub(crate) async fn released(pool: &FileRequests) {
    drop(
        tokio::time::timeout(Duration::from_secs(5), pool.slots.acquire())
            .await
            .unwrap()
            .unwrap(),
    );
}

#[test]
fn invalid_budgets_cannot_create_admission_pool() {
    let valid = limits();
    let mut cases = vec![valid; 10];
    cases[0].in_flight = 0;
    cases[1].in_flight = Semaphore::MAX_PERMITS + 1;
    cases[2].timeout = Duration::ZERO;
    cases[3].timeout = Duration::MAX;
    cases[4].query.fields = 0;
    cases[5].query.raw_bytes = usize::MAX;
    cases[6].authorization.headers.bytes = 0;
    cases[7].authorization.query_bytes = 0;
    cases[8].rejection.wire_bytes = 0;
    cases[9].rejection.chunk_bytes = usize::MAX;
    for limits in cases {
        assert!(matches!(
            FileRequests::new(limits),
            Err(AdmissionError::InvalidConfiguration)
        ));
    }
}

#[tokio::test]
async fn shared_pool_has_no_queue_and_capability_retains_original_slot() {
    let session = session().await;
    let pool = FileRequests::new(limits()).unwrap();
    let shared = pool.clone();
    for (method, operation) in [
        (Method::GET, auth::Operation::Read),
        (Method::HEAD, auth::Operation::Read),
        (Method::POST, auth::Operation::Write),
    ] {
        let admitted = pool.admit().unwrap();
        let deadline = admitted.deadline();
        full(&shared);
        let auth = admitted
            .authorize(&session, &method, &headers(), b"path=%2Ffile&username=root")
            .await
            .unwrap();
        assert_eq!(auth.deadline(), deadline);
        assert_eq!(auth.operation(), operation);
        assert_eq!(auth.path(), Some("/file"));
        full(&shared);
        drop(auth);
        drop(shared.admit().unwrap());
    }
    let admitted = pool.admit().unwrap();
    drop(admitted);
    drop(shared.admit().unwrap());
}

#[tokio::test]
async fn binding_precedes_auth_and_header_and_query_budgets_precede_binding() {
    let session = session().await;
    let pool = FileRequests::new(limits()).unwrap();
    let empty = HeaderMap::new();
    let rejected = pool
        .admit()
        .unwrap()
        .authorize(&session, &Method::GET, &empty, b"path=a&path=b")
        .await
        .err()
        .unwrap();
    assert_eq!(
        rejected.error(),
        &Error::Query(file_query::BindingError::Duplicate(
            sandproof_guest_runtime::file_query::Parameter::Path
        ))
    );
    full(&pool);
    drop(rejected);
    let mut large = empty.clone();
    large.insert("ignored", "x".repeat(1024).parse().unwrap());
    let rejected = pool
        .admit()
        .unwrap()
        .authorize(&session, &Method::GET, &large, b"path=a&path=b")
        .await
        .err()
        .unwrap();
    assert_eq!(rejected.error(), &Error::HeaderLimit);
    assert_eq!(
        rejected.into_response().await.err().unwrap(),
        ResponseError::Unmapped(Error::HeaderLimit)
    );
    let rejected = pool
        .admit()
        .unwrap()
        .authorize(&session, &Method::GET, &empty, &vec![b'x'; 1025])
        .await
        .err()
        .unwrap();
    assert_eq!(
        rejected.error(),
        &Error::Query(file_query::BindingError::ResourceExhausted)
    );
    drop(rejected);
    let rejected = pool
        .admit()
        .unwrap()
        .authorize(&session, &Method::DELETE, &headers(), b"")
        .await
        .err()
        .unwrap();
    assert_eq!(rejected.error(), &Error::UnsupportedMethod);
}

#[tokio::test]
async fn error_response_and_last_bytes_clone_retain_admission() {
    let session = session().await;
    let pool = FileRequests::new(limits()).unwrap();
    for (query, status, expected) in [
        (b"path=a&path=b".as_slice(), StatusCode::BAD_REQUEST,
            b"Invalid format for parameter path: multiple values for single value parameter 'path'\n".as_slice()),
        (b"path=a", StatusCode::UNAUTHORIZED,
            b"{\"code\":401,\"message\":\"missing signature query parameter\"}\n"),
    ] {
        let admitted = pool.admit().unwrap();
        let owner = Arc::downgrade(&admitted.lease);
        let rejected = admitted.authorize(&session, &Method::GET, &HeaderMap::new(), query)
            .await.err().unwrap();
        full(&pool);
        let response = rejected.into_response().await.unwrap();
        assert_eq!(response.status(), status);
        full(&pool);
        let mut body = response.into_body();
        let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
        assert_eq!(bytes.as_ref(), expected);
        let retained = bytes.clone();
        drop(bytes);
        drop(body);
        // Observe the blocking sender releasing its distinct owner. With just
        // the final byte owner left, saturation cannot be a sender-exit race.
        tokio::time::timeout(Duration::from_secs(5), async {
            while owner.strong_count() > 1 {
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        assert_eq!(owner.strong_count(), 1);
        full(&pool);
        drop(retained);
        released(&pool).await;
    }
}

#[tokio::test]
async fn expired_preparation_and_rejection_delivery_never_restart_deadline() {
    let session = session().await;
    let pool = FileRequests::new(limits()).unwrap();
    let admitted = pool.admit().unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    let rejected = admitted
        .authorize(&session, &Method::GET, &headers(), b"path=a&path=b")
        .await
        .err()
        .unwrap();
    assert_eq!(rejected.error(), &Error::DeadlineExceeded);
    full(&pool);
    drop(rejected);
    let rejected = pool
        .admit()
        .unwrap()
        .authorize(&session, &Method::GET, &HeaderMap::new(), b"")
        .await
        .err()
        .unwrap();
    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::time::resume();
    assert_eq!(
        rejected.into_response().await.err().unwrap(),
        ResponseError::Delivery(upload_response::Error::DeadlineExceeded)
    );
    released(&pool).await;
}

#[tokio::test]
async fn failed_error_encoding_does_not_leak_request_slot() {
    let session = session().await;
    let mut limits = limits();
    limits.rejection.wire_bytes = 1;
    let pool = FileRequests::new(limits).unwrap();
    let rejected = pool
        .admit()
        .unwrap()
        .authorize(&session, &Method::GET, &HeaderMap::new(), b"")
        .await
        .err()
        .unwrap();
    assert_eq!(
        rejected.into_response().await.err().unwrap(),
        ResponseError::Delivery(upload_response::Error::ResponseLimit)
    );
    released(&pool).await;
}

#[tokio::test]
async fn head_rejections_keep_get_headers_validate_budget_and_release_only_on_finish() {
    let session = session().await;
    for query in [
        b"path=a&path=b".as_slice(),
        b"signature_expiration=%FF%0A",
        b"",
    ] {
        let pool = FileRequests::new(limits()).unwrap();
        let get = pool
            .admit()
            .unwrap()
            .authorize(&session, &Method::GET, &HeaderMap::new(), query)
            .await
            .err()
            .unwrap()
            .into_response()
            .await
            .unwrap();
        let (parts, mut body) = get.into_parts();
        let mut size = 0;
        while let Some(frame) = body.frame().await {
            // Original owned-body backpressure requires releasing each chunk
            // before asking for EOF; BodyExt::collect retains those chunks.
            size += frame.unwrap().into_data().unwrap().len();
        }
        released(&pool).await;
        for finish in 0..3 {
            let response = pool
                .admit()
                .unwrap()
                .authorize(&session, &Method::HEAD, &HeaderMap::new(), query)
                .await
                .err()
                .unwrap()
                .into_response()
                .await
                .unwrap();
            assert_eq!(response.status(), parts.status);
            assert_eq!(response.headers(), &parts.headers);
            full(&pool);
            let mut body = response.into_body();
            match finish {
                0 => assert!(body.frame().await.is_none()),
                1 => {} // Dropping an unpolled HEAD must also release its sender.
                _ => {
                    tokio::time::pause();
                    tokio::time::advance(Duration::from_secs(31)).await;
                    assert!(body.frame().await.unwrap().is_err());
                    tokio::time::resume();
                }
            }
            drop(body);
            released(&pool).await;
        }
        for budget in [size, size - 1] {
            let mut limits = limits();
            limits.rejection.wire_bytes = budget;
            let pool = FileRequests::new(limits).unwrap();
            let response = pool
                .admit()
                .unwrap()
                .authorize(&session, &Method::HEAD, &HeaderMap::new(), query)
                .await
                .err()
                .unwrap()
                .into_response()
                .await;
            if budget == size {
                assert!(response.unwrap().into_body().frame().await.is_none());
            } else {
                assert_eq!(
                    response.err(),
                    Some(ResponseError::Delivery(
                        upload_response::Error::ResponseLimit
                    ))
                );
            }
            released(&pool).await;
        }
    }
}
