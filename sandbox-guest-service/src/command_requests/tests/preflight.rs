use super::*;
use crate::command::{CommandBodyLimits, PidRegistry, RegistryError, RegistryLimits};
use crate::{AccountPool, AccountSource, ContextError, ContextLimits};
use http_body_util::BodyExt;
use sandproof_guest_runtime::{
    Credentials,
    accounts::{Account, AccountError},
    pid_claims::Binding,
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "start.rs"]
mod start;

#[path = "account.rs"]
mod account;
#[path = "http.rs"]
mod http;

struct Accounts(Arc<AtomicUsize>);
impl AccountSource for Accounts {
    fn lookup(&self, name: &str) -> Result<Account, AccountError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        if name != "alice" {
            return Err(AccountError::NotFound);
        }
        Ok(Account {
            name: name.into(),
            home: "/home/alice".into(),
            credentials: Credentials {
                uid: 1001,
                gid: 1002,
                groups: vec![],
            },
        })
    }
}
fn context_limits() -> ContextLimits {
    ContextLimits {
        bytes: 4096,
        env_entries: 32,
        arguments: 32,
        username_bytes: 64,
        authorization_bytes: 256,
    }
}
fn response_limits() -> CommandBodyLimits {
    CommandBodyLimits {
        message_bytes: 4096,
        keepalive: Duration::from_secs(10),
    }
}
fn registry() -> PidRegistry {
    PidRegistry::new(
        Binding::new([1; 32]).unwrap(),
        1,
        RegistryLimits {
            records: 1,
            metadata_bytes: 4096,
            retention: Duration::from_secs(30),
        },
    )
    .unwrap()
}
async fn prepared(
    pool: &CommandRequests,
    session: &Session<Meta>,
    accounts: &AccountPool<Accounts>,
) -> Prepared {
    authenticate(pool, session)
        .await
        .prepare(accounts, context_limits())
        .await
        .unwrap()
}

#[tokio::test]
async fn explicit_basic_precedes_decode_and_default_lookup_is_deferred() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let accounts = AccountPool::new(Accounts(calls.clone()), 1, Duration::from_secs(30)).unwrap();
    for basic in [
        None,
        Some("Basic malformed"),
        Some("Basic YWxpY2U6"),
        Some("Basic bWlzc2luZzo="),
    ] {
        let mut headers = headers();
        if let Some(basic) = basic {
            headers.insert("authorization", basic.parse().unwrap());
        }
        let auth = pool
            .admit()
            .unwrap()
            .authenticate(&session, headers)
            .await
            .unwrap();
        let prepared = auth.prepare(&accounts, context_limits()).await;
        full(&pool);
        if basic == Some("Basic bWlzc2luZzo=") {
            assert_eq!(
                prepared.err().unwrap().error(),
                Error::Context(ContextError::Unauthenticated)
            );
        } else {
            let failure = prepared
                .unwrap()
                .decode(
                    "List",
                    Body::from("invalid"),
                    Instant::now() + Duration::from_secs(30),
                )
                .await
                .err()
                .unwrap();
            assert_eq!(
                failure.error(),
                Error::Protocol(ProtocolError::InvalidArgument)
            );
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

struct Blocked {
    gate: Arc<(Mutex<bool>, Condvar)>,
    entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}
impl AccountSource for Blocked {
    fn lookup(&self, name: &str) -> Result<Account, AccountError> {
        self.entered
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        let _guard = self
            .gate
            .1
            .wait_while(self.gate.0.lock().unwrap(), |open| !*open)
            .unwrap();
        Accounts(Arc::new(AtomicUsize::new(0))).lookup(name)
    }
}

#[tokio::test]
async fn timed_out_account_lookup_holds_request_and_auth_until_actual_exit() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    let gate = Gate::new();
    let (entered, started) = tokio::sync::oneshot::channel();
    let accounts = AccountPool::new(
        Blocked {
            gate: gate.0.clone(),
            entered: Mutex::new(Some(entered)),
        },
        1,
        Duration::from_mins(1),
    )
    .unwrap();
    let mut headers = headers();
    headers.insert("authorization", "Basic YWxpY2U6".parse().unwrap());
    let auth = pool
        .admit()
        .unwrap()
        .authenticate(&session, headers)
        .await
        .unwrap();
    let deadline = auth.deadline();
    let task = tokio::spawn(async move {
        auth.prepare_http(&axum::http::Method::POST, &accounts, context_limits())
            .await
    });
    started.await.unwrap();
    tokio::time::pause();
    // Tokio rounds timer deadlines up to milliseconds. This deliberately
    // blocked worker also inhibits paused-clock auto-advance, so stopping
    // exactly at the original Instant can leave the timeout asleep forever.
    tokio::time::advance(
        deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
    )
    .await;
    tokio::time::resume();
    // Bound failure of the test itself without changing the request deadline
    // or releasing the blocked worker before the retention assertions.
    let failure = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("original account deadline did not fire")
        .unwrap()
        .err()
        .unwrap();
    assert_eq!(
        failure.error(),
        Error::Context(ContextError::DeadlineExceeded)
    );
    assert_eq!(failure.deadline(), deadline);
    drop(failure);
    full(&pool);
    let mut init = Box::pin(session.initialize(br#"{"accessToken":"token"}"#));
    pending(init.as_mut()).await;
    gate.release();
    released(&pool).await;
    init.await.unwrap();
}

#[tokio::test]
async fn unary_response_clones_keep_capacity_but_release_authorization() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    let accounts = AccountPool::new(
        Accounts(Arc::new(AtomicUsize::new(0))),
        1,
        Duration::from_secs(30),
    )
    .unwrap();
    let ready = prepared(&pool, &session, &accounts)
        .await
        .decode(
            "List",
            Body::from("{}"),
            Instant::now() + Duration::from_mins(1),
        )
        .await
        .unwrap();
    let reply = ready
        .execute_pid(&registry(), response_limits())
        .await
        .unwrap();
    assert!(!reply.is_stream());
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"changed"}"#)
        .await
        .unwrap();
    let mut body = reply.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    assert_eq!(bytes, "{}");
    let cloned = bytes.clone();
    drop(bytes);
    drop(body);
    full(&pool);
    drop(cloned);
    assert!(pool.admit().is_ok());
}

#[tokio::test]
async fn missing_pid_or_tag_precedes_control_arguments_and_start_uses_its_own_dispatch() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    let accounts = AccountPool::new(
        Accounts(Arc::new(AtomicUsize::new(0))),
        1,
        Duration::from_secs(30),
    )
    .unwrap();
    let registry = registry();
    for method in ["SendInput", "SendSignal", "Update", "CloseStdin"] {
        let ready = prepared(&pool, &session, &accounts)
            .await
            .decode(
                method,
                Body::from(r#"{"process":{"pid":1}}"#),
                Instant::now() + Duration::from_mins(1),
            )
            .await
            .unwrap();
        let failure = ready
            .execute_pid(&registry, response_limits())
            .await
            .err()
            .unwrap();
        assert_eq!(failure.error(), Error::Registry(RegistryError::NotFound));
        full(&pool);
    }
    for method in [
        "SendInput",
        "SendSignal",
        "Update",
        "CloseStdin",
        "Connect",
        "Start",
    ] {
        let body = if matches!(method, "Start" | "Connect") {
            framed(br#"{"process":{"tag":"tag"}}"#)
        } else {
            Body::from(r#"{"process":{"tag":"tag"}}"#)
        };
        let ready = prepared(&pool, &session, &accounts)
            .await
            .decode(method, body, Instant::now() + Duration::from_mins(1))
            .await
            .unwrap();
        assert_eq!(
            ready
                .execute_pid(&registry, response_limits())
                .await
                .err()
                .unwrap()
                .error(),
            if method == "Start" {
                Error::UnsupportedOperation
            } else {
                Error::Registry(RegistryError::NotFound)
            }
        );
    }
}

#[tokio::test]
async fn an_unpolled_unary_body_cannot_outlive_the_admitted_deadline() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    let accounts = AccountPool::new(
        Accounts(Arc::new(AtomicUsize::new(0))),
        1,
        Duration::from_secs(30),
    )
    .unwrap();
    let ready = prepared(&pool, &session, &accounts)
        .await
        .decode(
            "List",
            Body::from("{}"),
            Instant::now() + Duration::from_secs(30),
        )
        .await
        .unwrap();
    let reply = ready
        .execute_pid(&registry(), response_limits())
        .await
        .unwrap();
    let deadline = reply.deadline();
    let mut body = reply.into_body();
    tokio::time::pause();
    tokio::time::advance(deadline.saturating_duration_since(Instant::now())).await;
    assert!(body.frame().await.unwrap().is_err());
    drop(body);
    assert!(pool.admit().is_ok());
}
