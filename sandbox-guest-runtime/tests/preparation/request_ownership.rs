use super::*;
use sandproof_guest_service::command::PreparationRequest;
use std::sync::Arc;
use tokio::sync::Semaphore;

fn request(slots: &Arc<Semaphore>, deadline: Instant) -> PreparationRequest {
    PreparationRequest {
        deadline,
        owner: Arc::new(slots.clone().try_acquire_owned().unwrap()),
    }
}
async fn admitted(fixture: &Fixture, request: PreparationRequest) -> PendingCommand {
    fixture
        .owners
        .prepare_command_admitted(
            fixture.spec(),
            fixture.session.authorize(Some("token")).await.unwrap(),
            ExecutionDeadline::from_duration(Instant::now(), Duration::ZERO).unwrap(),
            limits(),
            request,
        )
        .unwrap()
}
async fn credit(slots: &Arc<Semaphore>) {
    drop(
        timeout(Duration::from_secs(5), slots.acquire())
            .await
            .unwrap()
            .unwrap(),
    );
}

#[tokio::test]
async fn request_expiry_caps_preparation_and_an_expired_request_cannot_spawn() {
    let fixture = Fixture::new().await;
    let slots = Arc::new(Semaphore::new(1));
    let result = fixture.owners.prepare_command_admitted(
        fixture.spec(),
        fixture.session.authorize(Some("token")).await.unwrap(),
        ExecutionDeadline::from_duration(Instant::now(), Duration::ZERO).unwrap(),
        limits(),
        request(&slots, Instant::now()),
    );
    assert!(matches!(result, Err(CommandError::DeadlineExceeded)));
    assert_eq!(slots.available_permits(), 1);
    fixture.assert_no_effect();
    assert_eq!(fixture.gate_count(), 0);

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut pending = admitted(&fixture, request(&slots, deadline)).await;
    let identity = pending.prepared().await.unwrap();
    let _guard = kernel_guard(identity);
    let mut observer = pending.observer();
    tokio::time::pause();
    tokio::time::advance(deadline.saturating_duration_since(Instant::now())).await;
    tokio::time::resume();
    assert_eq!(stopped(&mut observer).await, CommandError::DeadlineExceeded);
    capacity_restored(&fixture.owners).await;
    // Actual process cleanup does not release the caller's still-owned ticket.
    assert_eq!(slots.available_permits(), 0);
    drop(pending);
    credit(&slots).await;
    fixture.assert_no_effect();
    assert_eq!(fixture.gate_count(), 0);
}

#[test]
fn queued_pid_work_keeps_request_credit_after_cancellation_and_actual_child_cleanup() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let fixture = Fixture::new().await;
        let slots = Arc::new(Semaphore::new(1));
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut pending = admitted(&fixture, request(&slots, deadline)).await;
        let identity = pending.prepared().await.unwrap();
        let _guard = kernel_guard(identity);
        let mut observer = pending.observer();
        let registrar = PidRegistrar::new(store(&fixture));
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        started.await.unwrap();
        let mut registering = Box::pin(pending.register_pid(registrar.clone(), binding()));
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(registering.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        drop(registering);
        tokio::time::pause();
        tokio::time::advance(deadline.saturating_duration_since(Instant::now())).await;
        tokio::time::resume();
        assert_eq!(stopped(&mut observer).await, CommandError::DeadlineExceeded);
        capacity_restored(&fixture.owners).await;
        assert_eq!(fixture.gate_count(), 0);
        assert_eq!(
            slots.available_permits(),
            0,
            "queued PID work still owns its request"
        );
        release.send(()).unwrap();
        blocker.await.unwrap();
        credit(&slots).await;
        fixture.assert_no_effect();
        drop(registrar);
        // Request credit and the registrar's file lock belong to different
        // closure fields; don't assume an order for their final destruction.
        let mut reopened = timeout(Duration::from_secs(5), async {
            loop {
                match reopen(&fixture) {
                    Ok(store) => break store,
                    Err(ClaimError::Busy) => tokio::task::yield_now().await,
                    Err(error) => panic!("unexpected reopen failure: {error:?}"),
                }
            }
        })
        .await
        .unwrap();
        reopened.claim(identity).unwrap();
    });
}

#[tokio::test]
async fn request_credit_follows_accepted_dispatch_until_actual_gate_release() {
    let fixture = Fixture::new().await;
    let slots = Arc::new(Semaphore::new(1));
    let mut pending = admitted(
        &fixture,
        request(&slots, Instant::now() + Duration::from_secs(30)),
    )
    .await;
    let identity = pending.prepared().await.unwrap();
    let guard = kernel_guard(identity);
    pidfd_send_signal(&guard.0, Signal::STOP).unwrap();
    assert!(
        waitid(
            WaitId::PidFd(guard.0.as_fd()),
            WaitIdOptions::STOPPED | WaitIdOptions::NOWAIT
        )
        .unwrap()
        .unwrap()
        .stopped()
    );
    let registered = pending
        .register_pid(PidRegistrar::new(store(&fixture)), binding())
        .await
        .unwrap();
    // This isolated fixture's original durable identity is its private selector
    // reservation, as in the existing registration tests; no public Start route.
    let (command, mut output) = registered.commit().await.unwrap();
    let command = Stop(command);
    assert_eq!(
        command.0.status().launch,
        sandproof_guest_service::command::LaunchStatus::Pending
    );
    assert_eq!(slots.available_permits(), 0);
    fixture.assert_no_effect();
    pidfd_send_signal(&guard.0, Signal::CONT).unwrap();
    command.0.started().await.unwrap();
    credit(&slots).await;
    // Only the request work settled. Background execution still owns its record.
    assert!(command.0.status().completion.is_none());
    assert!(matches!(
        fixture.owners.reserve(),
        Err(CommandError::ResourceExhausted)
    ));
    assert!(matches!(
        output.next().await,
        Some(Ok(OutputEvent::Data(_)))
    ));
    command.0.close_input(identity).unwrap();
    assert_eq!(command.0.wait().await.unwrap().code, 0);
    drop(output);
    drop(command);
    capacity_restored(&fixture.owners).await;
    assert_eq!(fixture.gate_count(), 0);
}
