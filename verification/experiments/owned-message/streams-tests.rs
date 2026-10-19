// Appended only to an isolated copy of the original streams.rs. No fake
// CommandOutput/Shared/channel implementation is substituted.
#[cfg(test)]
pub(in crate::command) mod owned_message_contract_tests {
    use super::*;
    use crate::command::{CommandStatus, LaunchStatus, OwnerLimits};
    use sandproof_guest_runtime::{LaunchSpec, ProcessLimits};
    use std::collections::BTreeMap;
    use std::future::{Future, poll_fn};
    use std::path::Path;
    use std::sync::Mutex;
    use std::task::Poll;
    use std::time::Duration;
    use tokio::sync::watch;

    pub(in crate::command) struct Rig {
        pub(in crate::command) command: ManagedCommand,
        pub(in crate::command) subscription: CommandSubscription,
        pub(in crate::command) bytes: Arc<Semaphore>,
        pub(in crate::command) subscribers: Arc<Semaphore>,
        pub(in crate::command) records: Arc<Semaphore>,
        _directory: tempfile::TempDir,
    }
    pub(in crate::command) async fn fixture() -> Rig {
        let directory = tempfile::tempdir().unwrap();
        let worker = std::env::var("SANDPROOF_OWNED_MESSAGE_WORKER").unwrap();
        let spec = LaunchSpec {
            envd_wrapper_path: None,
            program: "/bin/true".into(),
            args: vec![],
            cwd: directory.path().into(),
            env: BTreeMap::new(),
            stdin: false,
            pty: None,
            credentials: None,
        };
        let mut process = crate::command::CommandProcess::prepare(
            Path::new(&worker),
            directory.path(),
            11,
            7,
            &spec,
            ProcessLimits {
                chunk_bytes: 4096,
                launch_bytes: 16384,
                handshake_timeout: Duration::from_secs(5),
            },
        )
        .await
        .unwrap();
        let identity = process.identity();
        let control = process.control();
        // No command driver/session/business is modelled by this fixture.
        // A real, privately owned helper is aborted and reaped before using
        // its retained control identity to construct the original Shared.
        process.abort_prepared().await.unwrap();
        drop(process);
        let records = Arc::new(Semaphore::new(1));
        let subscribers = Arc::new(Semaphore::new(1));
        let shared = Arc::new(Shared {
            identity,
            control,
            stdin_disabled: true,
            status: watch::channel(CommandStatus {
                launch: LaunchStatus::Pending,
                completion: None,
                completed_at: None,
                deadline_action: None,
                owner_lost: false,
            })
            .0,
            output_wait_cancelled: watch::channel(false).0,
            streams: Mutex::new(Subscribers::default()),
            subscriber_slots: Arc::clone(&subscribers),
            input: tokio::sync::Mutex::new(None),
            limits: OwnerLimits {
                records: 1,
                subscribers: 1,
                queued_chunks: 1,
                output_bytes: 8,
                input_chunk_bytes: 8,
            },
            record: Mutex::new(Some(Arc::clone(&records).try_acquire_owned().unwrap())),
        });
        let command = ManagedCommand { shared };
        let subscription = command.subscribe(identity).unwrap();
        let bytes = Arc::clone(&command.shared.streams.lock().unwrap().live[0].bytes);
        Rig {
            command,
            subscription,
            bytes,
            subscribers,
            records,
            _directory: directory,
        }
    }
    pub(in crate::command) fn publish(command: &ManagedCommand, bytes: &[u8]) {
        command.shared.streams.lock().unwrap().publish(
            &command.shared,
            Channel::Stdout,
            bytes.to_vec(),
        );
    }
    pub(in crate::command) fn finish(command: &ManagedCommand) {
        command
            .shared
            .streams
            .lock()
            .unwrap()
            .finish(Ok(CommandExit {
                code: 0,
                signal: None,
                core_dumped: false,
            }));
    }
    pub(in crate::command) async fn take(subscription: &mut CommandSubscription) -> CommandOutput {
        match subscription.next().await.unwrap().unwrap() {
            OutputEvent::Data(output) => output,
            OutputEvent::End(_) => panic!("expected the original queued value"),
        }
    }
    #[tokio::test]
    async fn queued_and_received_original_output_keeps_all_three_owners() {
        let mut rig = fixture().await;
        publish(&rig.command, b"abcd");
        assert_eq!(rig.bytes.available_permits(), 4);
        rig.subscription.data.close();
        assert_eq!(
            rig.bytes.available_permits(),
            4,
            "close is not payload destruction"
        );
        let output = take(&mut rig.subscription).await;
        assert_eq!(output.bytes(), b"abcd");
        assert_eq!(output.channel(), Channel::Stdout);
        finish(&rig.command);
        drop(rig.subscription);
        drop(rig.command);
        assert_eq!(
            rig.bytes.available_permits(),
            4,
            "dequeue is not byte refund"
        );
        assert_eq!(
            rig.subscribers.available_permits(),
            0,
            "output retains shared subscriber permit"
        );
        assert_eq!(
            rig.records.available_permits(),
            0,
            "output retains original Shared"
        );
        drop(output);
        assert_eq!(rig.bytes.available_permits(), 8);
        assert_eq!(rig.subscribers.available_permits(), 1);
        assert_eq!(rig.records.available_permits(), 1);
    }
    #[tokio::test]
    async fn cancelled_borrowed_receive_does_not_consume_next_message() {
        let mut rig = fixture().await;
        {
            let waiting = rig.subscription.next();
            tokio::pin!(waiting);
            assert!(
                poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx)))
                    .await
                    .is_pending()
            );
        }
        publish(&rig.command, b"retained");
        let output = take(&mut rig.subscription).await;
        assert_eq!(output.bytes(), b"retained");
        assert_eq!(rig.bytes.available_permits(), 0);
        finish(&rig.command);
        drop(output);
    }
    #[tokio::test]
    async fn full_send_returns_and_drops_only_rejected_original_output() {
        let mut rig = fixture().await;
        publish(&rig.command, b"abcd");
        publish(&rig.command, b"xy");
        assert_eq!(
            rig.bytes.available_permits(),
            4,
            "Full returns rejected argument, queued prefix stays held"
        );
        let output = take(&mut rig.subscription).await;
        assert_eq!(output.bytes(), b"abcd");
        drop(output);
        assert_eq!(rig.bytes.available_permits(), 8);
        assert!(matches!(
            rig.subscription.next().await,
            Some(Err(CommandError::Lagged))
        ));
    }
    #[tokio::test]
    async fn receiver_drop_drops_queue_but_not_an_already_received_output() {
        let mut rig = fixture().await;
        publish(&rig.command, b"abc");
        let held = take(&mut rig.subscription).await;
        publish(&rig.command, b"de");
        assert_eq!(rig.bytes.available_permits(), 3);
        drop(rig.subscription);
        assert_eq!(
            rig.bytes.available_permits(),
            5,
            "only the queued second object was destroyed"
        );
        publish(&rig.command, b"ignored after receiver loss");
        assert_eq!(rig.bytes.available_permits(), 5);
        drop(rig.command);
        assert_eq!(rig.subscribers.available_permits(), 0);
        drop(held);
        assert_eq!(rig.bytes.available_permits(), 8);
        assert_eq!(rig.subscribers.available_permits(), 1);
        assert_eq!(rig.records.available_permits(), 1);
    }
    #[tokio::test]
    async fn freed_queue_slot_does_not_refund_held_byte_budget() {
        let mut rig = fixture().await;
        publish(&rig.command, b"12345678");
        let held = take(&mut rig.subscription).await;
        assert_eq!(rig.bytes.available_permits(), 0);
        publish(&rig.command, b"x");
        assert!(matches!(
            rig.subscription.next().await,
            Some(Err(CommandError::Lagged))
        ));
        assert_eq!(
            rig.bytes.available_permits(),
            0,
            "free queue slot did not free original held bytes"
        );
        drop(held);
        assert_eq!(rig.bytes.available_permits(), 8);
    }
    #[tokio::test]
    async fn zero_byte_message_still_retains_shared_owners() {
        let mut rig = fixture().await;
        publish(&rig.command, b"");
        let held = take(&mut rig.subscription).await;
        assert!(held.bytes().is_empty());
        finish(&rig.command);
        drop(rig.subscription);
        drop(rig.command);
        assert_eq!(rig.bytes.available_permits(), 8);
        assert_eq!(rig.subscribers.available_permits(), 0);
        assert_eq!(rig.records.available_permits(), 0);
        drop(held);
        assert_eq!(rig.subscribers.available_permits(), 1);
        assert_eq!(rig.records.available_permits(), 1);
    }
    #[tokio::test]
    async fn closed_try_send_returns_same_owned_object() {
        let mut rig = fixture().await;
        publish(&rig.command, b"abcd");
        let output = take(&mut rig.subscription).await;
        let CommandOutput {
            _record: record,
            _subscriber: subscriber,
            ..
        } = &output;
        let original_record = Arc::as_ptr(record);
        let original_subscriber = Arc::as_ptr(subscriber);
        let (sender, receiver) = mpsc::channel(1);
        drop(receiver);
        let error = sender.try_send(output).unwrap_err();
        assert!(matches!(error, mpsc::error::TrySendError::Closed(_)));
        let returned = error.into_inner();
        let CommandOutput {
            _record: record,
            _subscriber: subscriber,
            ..
        } = &returned;
        assert_eq!(Arc::as_ptr(record), original_record);
        assert_eq!(Arc::as_ptr(subscriber), original_subscriber);
        assert_eq!(returned.bytes(), b"abcd");
        assert_eq!(rig.bytes.available_permits(), 4);
        drop(returned);
        assert_eq!(rig.bytes.available_permits(), 8);
        finish(&rig.command);
    }
    #[test]
    fn zero_strong_references_does_not_mean_inner_resource_returned() {
        struct PausedDrop {
            _permit: OwnedSemaphorePermit,
            entered: std::sync::mpsc::Sender<()>,
            resume: Mutex<std::sync::mpsc::Receiver<()>>,
        }
        impl Drop for PausedDrop {
            fn drop(&mut self) {
                let _ = self.entered.send(());
                if let Ok(receiver) = self.resume.get_mut() {
                    let _ = receiver.recv();
                }
            }
        }
        let pool = Arc::new(Semaphore::new(1));
        let (entered, observe) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let owner = Arc::new(PausedDrop {
            _permit: Arc::clone(&pool).try_acquire_owned().unwrap(),
            entered,
            resume: Mutex::new(resume),
        });
        let weak = Arc::downgrade(&owner);
        let dropping = std::thread::spawn(move || drop(owner));
        observe.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(weak.upgrade().is_none());
        assert_eq!(weak.strong_count(), 0);
        assert_eq!(
            pool.available_permits(),
            0,
            "zero Arc references is not a permit refund"
        );
        release.send(()).unwrap();
        dropping.join().unwrap();
        assert_eq!(pool.available_permits(), 1);
    }
    #[tokio::test]
    async fn oneshot_original_payload_error_is_not_transport_loss() {
        let (mut tx, mut rx) = (None, None);
        let (send, receive) = oneshot::channel();
        tx.replace(send);
        rx.replace(receive);
        let mut subscriber = Subscriber {
            data: mpsc::channel(1).0,
            end: tx,
            bytes: Arc::new(Semaphore::new(8)),
            slot: Arc::new(Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap()),
        };
        let mut receiver = rx.unwrap();
        {
            let waiting = &mut receiver;
            tokio::pin!(waiting);
            assert!(
                poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx)))
                    .await
                    .is_pending()
            );
        }
        subscriber.finish(Err(CommandError::Lagged));
        subscriber.finish(Err(CommandError::Unavailable));
        receiver.close();
        assert!(matches!(receiver.await, Ok(Err(CommandError::Lagged))));
        let (send, receive) = oneshot::channel::<Result<CommandExit, CommandError>>();
        drop(send);
        assert!(receive.await.is_err());
        let (send, receive) = oneshot::channel::<Result<CommandExit, CommandError>>();
        drop(receive);
        let original = Ok(CommandExit {
            code: 17,
            signal: None,
            core_dumped: false,
        });
        assert_eq!(send.send(original).unwrap_err(), original);
    }
}
