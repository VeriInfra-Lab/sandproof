use super::*;
use crate::command_requests::http::{Rpc, reply};
use axum::http::Method;

fn reply_limits() -> reply::Limits {
    reply::Limits {
        wire_bytes: 4096,
        chunk_bytes: 4096,
    }
}
async fn selected_start<S: AccountSource>(
    pool: &CommandRequests,
    session: &Session<Meta>,
    accounts: &AccountPool<S>,
    content_type: &str,
    payload: &[u8],
) -> Ready {
    let mut headers = headers();
    headers.insert("content-type", content_type.parse().unwrap());
    pool.admit()
        .unwrap()
        .authenticate_http(session, &Method::POST, headers)
        .await
        .unwrap()
        .prepare_http(&Method::POST, accounts, context_limits())
        .await
        .unwrap()
        .select_json(&Method::POST, Rpc::Start)
        .unwrap()
        .decode_identity(framed(payload))
        .await
        .unwrap()
}

#[tokio::test]
async fn selected_start_keeps_default_observation_format_and_capacity_without_spawning() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    let fixture = StartFixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let accounts = AccountPool::new(Accounts(calls.clone()), 1, Duration::from_secs(30)).unwrap();
    for user in ["missing", "missing:invalid"] {
        let init =
            serde_json::to_vec(&serde_json::json!({"accessToken":"token", "defaultUser":user}))
                .unwrap();
        session.initialize(&init).await.unwrap();
        for content_type in [
            "application/connect+json",
            "application/connect+json; charset=utf-8",
        ] {
            for payload in [b"".as_slice(), b"{}", br#"{"process":{"cmd":"/bin/true"}}"#] {
                let before = calls.load(Ordering::SeqCst);
                let ready = selected_start(&pool, &session, &accounts, content_type, payload).await;
                assert_eq!(calls.load(Ordering::SeqCst), before);
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
                    Error::Context(ContextError::Unauthenticated)
                );
                assert_eq!(failure.deadline(), deadline);
                assert_eq!(
                    calls.load(Ordering::SeqCst),
                    before + usize::from(user == "missing")
                );
                let response = failure
                    .into_default_account_response(reply_limits())
                    .await
                    .unwrap();
                assert_eq!(response.status(), 200);
                assert_eq!(response.headers()["content-type"], content_type);
                assert_eq!(response.headers()["connect-accept-encoding"], "gzip");
                assert!(!response.headers().contains_key("content-length"));
                session.initialize(&init).await.unwrap();
                let mut body = response.into_body();
                let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
                assert_eq!(bytes[0], 2);
                assert_eq!(
                    u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize,
                    bytes.len() - 5
                );
                let decoded: serde_json::Value = serde_json::from_slice(&bytes[5..]).unwrap();
                assert_eq!(decoded["error"]["code"], "unauthenticated");
                assert_eq!(
                    decoded["error"]["message"],
                    format!("invalid default user: '{user}'")
                );
                let copy = bytes.clone();
                drop(bytes);
                drop(body);
                full(&pool);
                assert_eq!(fixture.registry.available_slots(), 1);
                drop(fixture.owners.reserve().unwrap());
                drop(copy);
                released(&pool).await;
            }
        }
    }
}

struct Fault(AccountError);
impl AccountSource for Fault {
    fn lookup(&self, _: &str) -> Result<Account, AccountError> {
        Err(self.0)
    }
}
#[tokio::test]
async fn default_source_faults_and_unselected_decoder_never_fabricate_a_stream_reply() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    let fixture = StartFixture::new();
    for fault in [
        AccountError::Unavailable,
        AccountError::ResourceExhausted,
        AccountError::InvalidArgument,
        AccountError::NotFound,
    ] {
        let accounts = AccountPool::new(Fault(fault), 1, Duration::from_secs(30)).unwrap();
        let ready = if fault == AccountError::NotFound {
            // An older lower-level caller has not selected any response format.
            start_ready(&pool, &session, &accounts, headers(), b"{}").await
        } else {
            selected_start(
                &pool,
                &session,
                &accounts,
                "application/connect+json",
                b"{}",
            )
            .await
        };
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
        let original = failure.error();
        let failure = failure
            .into_default_account_response(reply_limits())
            .await
            .unwrap_err();
        assert_eq!(failure.error(), reply::Error::UnclassifiedFault);
        assert_eq!(failure.request_error(), original);
        assert_eq!(failure.deadline(), deadline);
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        full(&pool);
        assert_eq!(fixture.registry.available_slots(), 1);
        drop(fixture.owners.reserve().unwrap());
        drop(failure);
        released(&pool).await;
    }
}
