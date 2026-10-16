use super::*;
use crate::command::{CommandOwners, ExecutionDeadline, OwnerLimits};
use crate::{InitLimits, Metadata, MetadataSource, ServiceError, Session};
use sandproof_guest_runtime::{LaunchSpec, ProcessLimits};
use sha2::{Digest, Sha512};
use std::collections::BTreeMap;
use std::future::{Future, poll_fn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Poll;
use std::time::Duration;
use tokio::sync::Semaphore;

struct MetadataFixture;
impl MetadataSource for MetadataFixture {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        Ok(Metadata {
            instance_id: "blocking-vm".into(),
            template_id: "blocking-template".into(),
            access_token_sha512: Sha512::digest(b"token").into(),
        })
    }
}

struct Fixture {
    commands: CommandOwners,
    requests: Arc<Semaphore>,
    session: Session<MetadataFixture>,
}
impl Fixture {
    async fn new() -> Self {
        let session = Session::new(
            MetadataFixture,
            InitLimits {
                body_bytes: 4096,
                token_bytes: 128,
                state_bytes: 2048,
                env_entries: 32,
                in_flight: 1,
                timeout: Duration::from_secs(5),
            },
        )
        .unwrap();
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        Self {
            commands: CommandOwners::new(OwnerLimits {
                records: 1,
                subscribers: 1,
                queued_chunks: 1,
                output_bytes: 4096,
                stdin_bytes: 4096,
            })
            .unwrap(),
            requests: Arc::new(Semaphore::new(1)),
            session,
        }
    }

    async fn admission(
        &self,
    ) -> (
        Context,
        AdmissionOwner,
        watch::Receiver<AdmissionStatus>,
        oneshot::Receiver<Reply>,
    ) {
        let (status, observer) = watch::channel(AdmissionStatus::Preparing);
        let (reply, receive) = oneshot::channel();
        let owner = AdmissionOwner::new(
            self.commands.reserve().unwrap(),
            status,
            reply,
            Arc::new(Mutex::new(None)),
        );
        let context = Context {
            authorization: self.session.authorize(Some("token")).await.unwrap(),
            execution: ExecutionDeadline::from_duration(Instant::now(), Duration::ZERO).unwrap(),
            until: Instant::now() + Duration::from_secs(30),
            limits: AdmissionLimits {
                ready_timeout: Duration::from_secs(30),
                cleanup_timeout: Duration::from_secs(5),
                retry_delay: Duration::from_secs(1),
            },
            request: Arc::new(self.requests.clone().try_acquire_owned().unwrap()),
        };
        (context, owner, observer, receive)
    }

    fn assert_owned(&self) {
        assert_eq!(self.requests.available_permits(), 0);
        assert!(matches!(
            self.commands.reserve(),
            Err(CommandError::ResourceExhausted)
        ));
    }

    async fn request_returned(&self) {
        drop(
            tokio::time::timeout(Duration::from_secs(5), self.requests.acquire())
                .await
                .unwrap()
                .unwrap(),
        );
    }
}

// The fault point below never calls the kernel spawn. Real worker, pipe, PTY,
// native-error and runtime-loss cases remain in the original runtime suite.
fn spec() -> PreparationSpec {
    PreparationSpec {
        worker: "/unused/worker".into(),
        private_directory: "/unused/gates".into(),
        session: 1,
        command: 1,
        launch: LaunchSpec {
            program: "/unused/user-code".into(),
            args: vec![],
            cwd: "/unused".into(),
            env: BTreeMap::new(),
            stdin: true,
            pty: None,
            credentials: None,
            envd_wrapper_path: None,
        },
        process_limits: ProcessLimits {
            chunk_bytes: 1024,
            launch_bytes: 4096,
            handshake_timeout: Duration::from_secs(5),
        },
    }
}

#[test]
fn queued_preparation_keeps_original_capacity_and_skips_cancelled_or_expired_work() {
    for cancellation in 0..4 {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        runtime.block_on(async {
            let fixture = Fixture::new().await;
            let (context, owner, mut observer, reply) = fixture.admission().await;
            let mut reply = Some(reply);
            let until = context.until;
            let (decision_sender, decision) = oneshot::channel();
            let (entered, started) = oneshot::channel();
            let (release, resume) = std::sync::mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                entered.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(10)).unwrap();
            });
            started.await.unwrap();
            let called = Arc::new(AtomicBool::new(false));
            let attempted = called.clone();
            let mut job = Some(Box::pin(prepare_and_run(
                spec(),
                context,
                decision,
                owner,
                move |_, _| {
                    attempted.store(true, Ordering::SeqCst);
                    panic!("cancelled/expired queue item reached spawn");
                },
            )));
            assert!(
                poll_fn(|cx| Poll::Ready(job.as_mut().unwrap().as_mut().poll(cx)))
                    .await
                    .is_pending()
            );
            match cancellation {
                0 => {
                    tokio::time::pause();
                    tokio::time::advance(until.saturating_duration_since(Instant::now())).await;
                    tokio::time::resume();
                }
                1 => drop(reply.take()),
                2 => drop(job.take()),
                3 => assert!(decision_sender.send(Decision::Abort).is_ok()),
                _ => unreachable!(),
            }
            fixture.assert_owned();
            assert_eq!(*observer.borrow(), AdmissionStatus::Preparing);
            assert!(!called.load(Ordering::SeqCst));
            release.send(()).unwrap();
            blocker.await.unwrap();
            if let Some(job) = job {
                tokio::time::timeout(Duration::from_secs(5), job)
                    .await
                    .unwrap();
            } else {
                tokio::time::timeout(Duration::from_secs(5), observer.changed())
                    .await
                    .unwrap()
                    .unwrap();
            }
            assert!(!called.load(Ordering::SeqCst));
            assert_eq!(
                *observer.borrow(),
                AdmissionStatus::Stopped(if cancellation != 0 {
                    CommandError::Cancelled
                } else {
                    CommandError::DeadlineExceeded
                })
            );
            fixture.request_returned().await;
            assert!(fixture.commands.reserve().is_ok());
        });
    }
}

#[test]
fn running_preparation_outlives_its_waiter_and_settles_or_quarantines_only_on_exit() {
    for panic in [false, true] {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        runtime.block_on(async {
            let fixture = Fixture::new().await;
            let (context, owner, mut observer, reply) = fixture.admission().await;
            let (_decision, decision) = oneshot::channel();
            let (entered, started) = oneshot::channel();
            let (release, resume) = std::sync::mpsc::channel();
            let caller_thread = std::thread::current().id();
            let job = tokio::spawn(prepare_and_run(
                spec(),
                context,
                decision,
                owner,
                move |_, _| {
                    entered.send(std::thread::current().id()).unwrap();
                    resume.recv_timeout(Duration::from_secs(10)).unwrap();
                    assert!(!panic, "controlled preparation panic");
                    Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "no child created",
                    ))
                },
            ));
            let blocking_thread = tokio::time::timeout(Duration::from_secs(5), started)
                .await
                .unwrap()
                .unwrap();
            assert_ne!(caller_thread, blocking_thread);
            job.abort();
            assert!(job.await.unwrap_err().is_cancelled());
            drop(reply);
            // Even loss of the async owner/waiter cannot drop the closure's
            // record, authorization or request while its real work is blocked.
            fixture.assert_owned();
            assert_eq!(*observer.borrow(), AdmissionStatus::Preparing);
            release.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(5), observer.changed())
                .await
                .unwrap()
                .unwrap();
            fixture.request_returned().await;
            if panic {
                assert_eq!(*observer.borrow(), AdmissionStatus::OwnerLost);
                assert!(matches!(
                    fixture.commands.reserve(),
                    Err(CommandError::ResourceExhausted)
                ));
            } else {
                assert_eq!(
                    *observer.borrow(),
                    AdmissionStatus::Stopped(CommandError::InvalidArgument)
                );
                assert!(fixture.commands.reserve().is_ok());
            }
        });
    }
}
