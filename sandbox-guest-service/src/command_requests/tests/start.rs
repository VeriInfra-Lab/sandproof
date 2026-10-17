use super::*;
use crate::{
    ExecutionSettings,
    command::{AdmissionLimits, CommandBodyError, CommandOwners, OwnerLimits, PidRegistrar},
    command_requests::StartPreparation,
};
use sandproof_guest_runtime::{ProcessLimits, pid_claims::PidClaims};
use std::{
    fs::File,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};

#[path = "default_account.rs"]
mod default_account;

#[path = "start/gzip.rs"]
mod gzip;

#[path = "start/expansion.rs"]
mod expansion;

#[path = "start/capacity.rs"]
mod capacity;

struct StartFixture {
    directory: tempfile::TempDir,
    owners: CommandOwners,
    registry: PidRegistry,
    registrar: PidRegistrar,
}
impl StartFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let uid = std::fs::metadata(directory.path()).unwrap().uid();
        let claims = PidClaims::create(
            File::open(directory.path()).unwrap(),
            uid,
            Binding::new([1; 32]).unwrap(),
            32,
        )
        .unwrap();
        Self {
            directory,
            owners: CommandOwners::new(OwnerLimits {
                records: 1,
                subscribers: 1,
                queued_chunks: 2,
                output_bytes: 1024,
                input_chunk_bytes: 1024,
            })
            .unwrap(),
            registry: registry(),
            registrar: PidRegistrar::new(claims),
        }
    }
    fn preparation(&self) -> StartPreparation<'_> {
        StartPreparation {
            owners: &self.owners,
            registry: &self.registry,
            registrar: &self.registrar,
            binding: Binding::new([1; 32]).unwrap(),
            session: 1,
            command: 1,
            worker: Path::new("/missing-start-preflight-test-worker"),
            private_directory: self.directory.path(),
            process_limits: ProcessLimits {
                chunk_bytes: 128,
                launch_bytes: 4096,
                handshake_timeout: Duration::from_secs(30),
            },
            admission_limits: AdmissionLimits {
                ready_timeout: Duration::from_secs(30),
                cleanup_timeout: Duration::from_secs(5),
                retry_delay: Duration::from_millis(10),
            },
        }
    }
}
fn settings() -> ExecutionSettings {
    ExecutionSettings {
        daemon_path: "/bin:/usr/bin".into(),
        daemon_workdir: "/".into(),
        limits: context_limits(),
    }
}
async fn start_ready<S: AccountSource>(
    pool: &CommandRequests,
    session: &Session<Meta>,
    accounts: &AccountPool<S>,
    headers: HeaderMap,
    payload: &[u8],
) -> Ready {
    let prepared = pool
        .admit()
        .unwrap()
        .authenticate(session, headers)
        .await
        .unwrap()
        .prepare(accounts, context_limits())
        .await
        .unwrap();
    let deadline = prepared.deadline();
    prepared
        .decode("Start", framed(payload), deadline)
        .await
        .unwrap()
}

#[tokio::test]
async fn start_reuses_explicit_account_and_only_resolves_defaults_once() {
    let session = session().await;
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"alice"}"#)
        .await
        .unwrap();
    let pool = CommandRequests::new(limits()).unwrap();
    let fixture = StartFixture::new();
    for basic in [None, Some("Basic malformed"), Some("Basic YWxpY2U6")] {
        let calls = Arc::new(AtomicUsize::new(0));
        let accounts =
            AccountPool::new(Accounts(calls.clone()), 1, Duration::from_secs(30)).unwrap();
        let mut headers = headers();
        if let Some(basic) = basic {
            headers.insert("authorization", basic.parse().unwrap());
        }
        let ready = start_ready(
            &pool,
            &session,
            &accounts,
            headers,
            br#"{"process":{"cmd":"/bin/true"}}"#,
        )
        .await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            usize::from(basic == Some("Basic YWxpY2U6"))
        );
        let failure = ready
            .start_pid(
                &accounts,
                &settings(),
                fixture.preparation(),
                CommandBodyLimits {
                    keepalive: Duration::ZERO,
                    ..response_limits()
                },
            )
            .await
            .err()
            .unwrap();
        assert_eq!(
            failure.error(),
            Error::Response(CommandBodyError::InvalidArgument)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.registry.available_slots(), 1);
        drop(fixture.owners.reserve().unwrap());
        full(&pool);
        drop(failure);
        released(&pool).await;
    }
}

#[tokio::test]
async fn default_account_precedes_invalid_command_and_tag_reservation() {
    let session = session().await;
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"missing"}"#)
        .await
        .unwrap();
    let pool = CommandRequests::new(limits()).unwrap();
    let fixture = StartFixture::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let accounts = AccountPool::new(Accounts(calls.clone()), 1, Duration::from_secs(30)).unwrap();
    for payload in [b"{}".as_slice(), br#"{"process":{"cmd":"/bin/true"}}"#] {
        let failure = start_ready(&pool, &session, &accounts, headers(), payload)
            .await
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
        drop(failure);
        released(&pool).await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    for payload in [br#"{"tag":""}"#.as_slice(), br#"{"tag":"same"}"#] {
        let failure = start_ready(&pool, &session, &accounts, headers(), payload)
            .await
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
        drop(failure);
        released(&pool).await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert_eq!(fixture.registry.available_slots(), 1);
    drop(fixture.owners.reserve().unwrap());
}

#[tokio::test]
async fn cancelled_and_expired_start_lookup_retains_original_auth_and_request_until_actual_exit() {
    for expire in [false, true] {
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
        let ready = start_ready(&pool, &session, &accounts, headers(), b"{}").await;
        let deadline = ready.deadline();
        let fixture = StartFixture::new();
        let task = tokio::spawn(async move {
            ready
                .start_pid(
                    &accounts,
                    &settings(),
                    fixture.preparation(),
                    response_limits(),
                )
                .await
        });
        started.await.unwrap();
        if expire {
            tokio::time::pause();
            tokio::time::advance(
                deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
            )
            .await;
            tokio::time::resume();
            let failure = tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .err()
                .unwrap();
            assert!(matches!(
                failure.error(),
                Error::DeadlineExceeded | Error::Context(ContextError::DeadlineExceeded)
            ));
            assert_eq!(failure.deadline(), deadline);
            drop(failure);
        } else {
            task.abort();
            assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        }
        full(&pool);
        let mut init = Box::pin(session.initialize(br#"{"accessToken":"token"}"#));
        pending(init.as_mut()).await;
        gate.release();
        released(&pool).await;
        init.await.unwrap();
    }
}
