use super::*;
use sandproof_guest_runtime::pid_claims::{Binding, Error as ClaimError, MAX_PID, PidClaims};
use sandproof_guest_service::command::{PidRegistrar, RegistrationError};
use std::{fs::File, os::unix::fs::MetadataExt};

fn binding() -> Binding {
    Binding::new([0x31; 32]).unwrap()
}
fn store(fixture: &Fixture) -> PidClaims {
    std::fs::set_permissions(
        fixture.directory.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    PidClaims::create(
        File::open(fixture.directory.path()).unwrap(),
        uid(fixture),
        binding(),
        MAX_PID,
    )
    .unwrap()
}
fn uid(fixture: &Fixture) -> u32 {
    std::fs::metadata(fixture.directory.path()).unwrap().uid()
}
fn reopen(fixture: &Fixture) -> Result<PidClaims, ClaimError> {
    PidClaims::open(
        File::open(fixture.directory.path()).unwrap(),
        uid(fixture),
        binding(),
        MAX_PID,
    )
}

#[tokio::test]
async fn durable_claim_precedes_real_exec_and_survives_process_completion_and_reopen() {
    let fixture = Fixture::new().await;
    let registrar = PidRegistrar::new(store(&fixture));
    let registered = fixture
        .pending(fixture.spec())
        .await
        .register_pid(registrar.clone(), binding())
        .await
        .unwrap();
    let identity = registered.identity();
    fixture.assert_no_effect();
    // This isolated fixture holds the exact identity reservation; production
    // active PID/tag lookup remains a separate registry obligation.
    let (command, mut stream) = registered.commit().await.unwrap();
    let command = Stop(command);
    command.0.started().await.unwrap();
    let OutputEvent::Data(data) = stream.next().await.unwrap().unwrap() else {
        panic!("missing ready");
    };
    assert_eq!(data.bytes(), b"ready");
    drop(data);
    assert_eq!(
        std::fs::read(fixture.directory.path().join("effect")).unwrap(),
        b"once"
    );
    command.0.close_input(identity).unwrap();
    assert_eq!(command.0.wait().await.unwrap().code, 0);
    assert!(matches!(stream.next().await, Some(Ok(OutputEvent::End(_)))));
    drop(stream);
    drop(command);
    capacity_restored(&fixture.owners).await;
    drop(registrar);
    assert!(matches!(
        reopen(&fixture).unwrap().claim(identity),
        Err(ClaimError::Reused)
    ));
    assert_eq!(fixture.gate_count(), 0);
}

#[tokio::test]
async fn reused_pid_wrong_binding_and_discarded_registration_never_execute() {
    for refusal in 0..3 {
        let fixture = Fixture::new().await;
        let mut claims = store(&fixture);
        let mut pending = fixture.pending(fixture.spec()).await;
        let identity = pending.prepared().await.unwrap();
        let mut observer = pending.observer();
        let guard = kernel_guard(identity);
        if refusal == 0 {
            claims.claim(identity).unwrap();
        }
        let registrar = PidRegistrar::new(claims);
        match refusal {
            0 => assert!(matches!(
                pending.register_pid(registrar.clone(), binding()).await,
                Err(RegistrationError::Storage(ClaimError::Reused))
            )),
            1 => assert!(matches!(
                pending
                    .register_pid(registrar.clone(), Binding::new([1; 32]).unwrap())
                    .await,
                Err(RegistrationError::Storage(ClaimError::BindingMismatch))
            )),
            2 => drop(
                pending
                    .register_pid(registrar.clone(), binding())
                    .await
                    .unwrap(),
            ),
            _ => unreachable!(),
        }
        assert_eq!(stopped(&mut observer).await, CommandError::Cancelled);
        assert_eq!(
            pidfd_send_signal(&guard.0, Signal::KILL),
            Err(rustix::io::Errno::SRCH)
        );
        fixture.assert_no_effect();
        assert_eq!(fixture.gate_count(), 0);
        capacity_restored(&fixture.owners).await;
        drop(registrar);
        let result = reopen(&fixture).unwrap().claim(identity);
        if refusal == 1 {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(ClaimError::Reused)));
        }
    }
}

#[test]
fn queued_claim_keeps_storage_capacity_after_cancellation_and_process_deadline() {
    for expire in [false, true] {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        runtime.block_on(async {
            let fixture = Fixture::new().await;
            let other = Fixture::new().await;
            let registrar = PidRegistrar::new(store(&fixture));
            let mut pending = fixture.pending(fixture.spec()).await;
            let identity = pending.prepared().await.unwrap();
            let _guard = kernel_guard(identity);
            let mut observer = pending.observer();
            let (occupied, started) = tokio::sync::oneshot::channel();
            let (release, gate) = std::sync::mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                occupied.send(()).unwrap();
                gate.recv().unwrap();
            });
            started.await.unwrap();
            let mut registering = Box::pin(pending.register_pid(registrar.clone(), binding()));
            assert!(
                std::future::poll_fn(|cx| Poll::Ready(registering.as_mut().poll(cx)))
                    .await
                    .is_pending()
            );
            drop(registering);
            fixture.assert_no_effect();
            assert_eq!(observer.status(), AdmissionStatus::Ready(identity));
            assert!(matches!(
                fixture.owners.reserve(),
                Err(CommandError::ResourceExhausted)
            ));
            if expire {
                tokio::time::pause();
                tokio::time::advance(Duration::from_secs(61)).await;
                tokio::time::resume();
                assert_eq!(stopped(&mut observer).await, CommandError::DeadlineExceeded);
                capacity_restored(&fixture.owners).await;
            }
            // A second owner cannot bypass the independent shared storage slot,
            // even after the first child was actually reaped and credited.
            let pending = other.pending(other.spec()).await;
            let mut rejected = pending.observer();
            assert!(matches!(
                pending.register_pid(registrar.clone(), binding()).await,
                Err(RegistrationError::Storage(ClaimError::Busy))
            ));
            assert_eq!(stopped(&mut rejected).await, CommandError::Cancelled);
            other.assert_no_effect();
            capacity_restored(&other.owners).await;
            release.send(()).unwrap();
            blocker.await.unwrap();
            if !expire {
                assert_eq!(stopped(&mut observer).await, CommandError::Cancelled);
            }
            fixture.assert_no_effect();
            assert_eq!(fixture.gate_count(), 0);
            capacity_restored(&fixture.owners).await;
            drop(registrar);
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
            // Cancelled queued work never started storage, so no PID was burned.
            reopened.claim(identity).unwrap();
        });
    }
}
