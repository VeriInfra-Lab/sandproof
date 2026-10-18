use super::*;
use crate::command_requests::http::{Rpc, reply};
use axum::http::Method;
use sandproof_guest_runtime::protocol::ProcessConfig;

async fn rejected(metadata_limit: bool) -> (Failure, CommandRequests, StartFixture) {
    let session = session().await;
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"alice"}"#)
        .await
        .unwrap();
    let pool = CommandRequests::new(limits()).unwrap();
    let mut fixture = StartFixture::new();
    if metadata_limit {
        fixture.registry = PidRegistry::new(
            Binding::new([1; 32]).unwrap(),
            1,
            RegistryLimits {
                records: 1,
                metadata_bytes: 1,
                retention: Duration::from_secs(30),
            },
        )
        .unwrap();
    }
    let occupied = (!metadata_limit).then(|| {
        fixture
            .registry
            .reserve(&ProcessConfig::default(), None)
            .unwrap()
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let accounts = AccountPool::new(Accounts(calls.clone()), 1, Duration::from_secs(30)).unwrap();
    let mut headers = headers();
    headers.insert("content-type", "application/connect+json".parse().unwrap());
    let ready = pool
        .admit()
        .unwrap()
        .authenticate_http(&session, &Method::POST, headers)
        .await
        .unwrap()
        .prepare_http(&Method::POST, &accounts, context_limits())
        .await
        .unwrap()
        .select_json(&Method::POST, Rpc::Start)
        .unwrap()
        .decode(framed(br#"{"process":{"cmd":"/bin/true"}}"#))
        .await
        .unwrap();
    let deadline = ready.deadline();
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
    assert_eq!(
        failure.error(),
        Error::Registry(RegistryError::ResourceExhausted)
    );
    assert_eq!(failure.deadline(), deadline);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // Both refusals precede the missing worker and consume no execution owner.
    drop(fixture.owners.reserve().unwrap());
    assert_eq!(
        fixture.registry.available_slots(),
        usize::from(metadata_limit)
    );
    drop(occupied);
    assert_eq!(fixture.registry.available_slots(), 1);
    full(&pool);
    (failure, pool, fixture)
}

fn reply_limits() -> reply::Limits {
    reply::Limits {
        wire_bytes: 4096,
        chunk_bytes: 4096,
    }
}

async fn check_rejection(metadata_limit: bool) {
    let (failure, pool, _fixture) = rejected(metadata_limit).await;
    let response = failure.into_start_response(reply_limits()).await.unwrap();
    assert_eq!(response.status(), 200);
    let mut body = response.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    assert_eq!(bytes[0], 2);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes[5..]).unwrap(),
        serde_json::json!({"error": {"code": "resource_exhausted",
            "message": "command registry capacity exceeded"}}),
    );
    let copy = bytes.clone();
    drop((bytes, body));
    full(&pool);
    drop(copy);
    released(&pool).await;
}

#[tokio::test]
async fn full_registry_rejects_start_before_worker_with_owned_capacity_reply() {
    check_rejection(false).await;
}

#[tokio::test]
async fn oversized_registry_metadata_rejects_start_before_worker_with_owned_capacity_reply() {
    check_rejection(true).await;
}

#[tokio::test]
async fn capacity_reply_requires_original_fact_error_and_deadline() {
    for case in 0..4 {
        let (mut failure, pool, _fixture) = rejected(false).await;
        let expected = match case {
            0 => {
                failure.start_rejection = None;
                reply::Error::UnclassifiedFault
            }
            1 => {
                failure.error = Error::Registry(RegistryError::Unavailable);
                reply::Error::UnclassifiedFault
            }
            2 => {
                failure.error = Error::DeadlineExceeded;
                reply::Error::UnclassifiedFault
            }
            _ => {
                failure.deadline = Instant::now();
                reply::Error::DeadlineExceeded
            }
        };
        let deadline = failure.deadline();
        let error = failure.error();
        let rejected = failure
            .into_start_response(reply_limits())
            .await
            .unwrap_err();
        assert_eq!(rejected.error(), expected);
        assert_eq!(rejected.deadline(), deadline);
        assert_eq!(rejected.request_error(), error);
        full(&pool);
        drop(rejected);
        released(&pool).await;
    }
}
