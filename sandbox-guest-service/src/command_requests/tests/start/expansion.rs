use super::*;
use crate::command_requests::http::start::Rejection;

#[tokio::test]
async fn expansion_retains_the_once_selected_account_and_does_not_reach_preparation() {
    let session = session().await;
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"alice"}"#)
        .await
        .unwrap();
    let pool = CommandRequests::new(limits()).unwrap();
    let fixture = StartFixture::new();
    for basic in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let accounts =
            AccountPool::new(Accounts(calls.clone()), 1, Duration::from_secs(30)).unwrap();
        let mut headers = headers();
        if basic {
            headers.insert("authorization", "Basic YWxpY2U6".parse().unwrap());
        }
        let ready = start_ready(
            &pool,
            &session,
            &accounts,
            headers,
            br#"{"process":{"cmd":"/bin/true","cwd":"~other"}}"#,
        )
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(basic));
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
            Error::Context(ContextError::InvalidArgument)
        );
        assert!(
            matches!(failure.start_rejection, Some(Rejection::Expansion(ref observed)) if *observed == crate::upload_response::PathExpansion::new("alice"))
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.registry.available_slots(), 1);
        drop(fixture.owners.reserve().unwrap());
        full(&pool);
        drop(failure);
        released(&pool).await;
    }
}
