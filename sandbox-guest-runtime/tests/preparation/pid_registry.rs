use super::*;
use sandproof_guest_runtime::protocol::ProcessConfig;
use sandproof_guest_service::command::{PidRegistry, RegistryError, RegistryLimits};

fn registry() -> PidRegistry {
    PidRegistry::new(
        binding(),
        1,
        RegistryLimits {
            records: 1,
            metadata_bytes: 4096,
            retention: Duration::from_secs(30),
        },
    )
    .unwrap()
}
fn config(fixture: &Fixture) -> ProcessConfig {
    let launch = fixture.spec().launch;
    ProcessConfig {
        cmd: launch.program,
        args: launch.args,
        envs: launch.env,
        cwd: Some(launch.cwd.to_str().unwrap().into()),
    }
}
async fn registry_capacity(registry: &PidRegistry) {
    timeout(Duration::from_secs(5), async {
        while registry.available_slots() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("registry retained a settled/expired record");
}

#[tokio::test]
async fn cancelled_before_registry_acceptance_aborts_original_child_and_burns_pid() {
    let fixture = Fixture::new().await;
    let registry = registry();
    let reservation = registry.reserve(&config(&fixture), None).unwrap();
    let registrar = PidRegistrar::new(store(&fixture));
    let ticket = fixture
        .pending(fixture.spec())
        .await
        .register_pid(registrar.clone(), binding())
        .await
        .unwrap();
    let identity = ticket.identity();
    let _guard = kernel_guard(identity);
    let mut observer = ticket.observer();
    let pending = reservation.publish(ticket).unwrap();
    assert_eq!(pending.identity(), identity);
    assert!(matches!(
        registry.live(identity.pid),
        Err(RegistryError::NotReady)
    ));
    assert_eq!(registry.available_slots(), 0);
    // On this current-thread runtime the registry owner has not been polled.
    drop(pending);
    assert_eq!(stopped(&mut observer).await, CommandError::Cancelled);
    registry_capacity(&registry).await;
    capacity_restored(&fixture.owners).await;
    fixture.assert_no_effect();
    assert_eq!(fixture.gate_count(), 0);
    assert!(matches!(
        registry.connectable(identity.pid),
        Err(RegistryError::NotFound)
    ));
    drop(registrar);
    assert!(matches!(
        reopen(&fixture).unwrap().claim(identity),
        Err(ClaimError::Reused)
    ));
}

#[tokio::test]
async fn lost_start_reply_keeps_original_control_and_terminal_retention_never_refreshes() {
    let fixture = Fixture::new().await;
    let registry = registry();
    let expected_config = config(&fixture);
    let reservation = registry.reserve(&expected_config, Some("")).unwrap();
    let registrar = PidRegistrar::new(store(&fixture));
    let ticket = fixture
        .pending(fixture.spec())
        .await
        .register_pid(registrar.clone(), binding())
        .await
        .unwrap();
    let identity = ticket.identity();
    let _guard = kernel_guard(identity);
    let pending = reservation.publish(ticket).unwrap();
    let command = Stop(wait_live(&registry, identity.pid).await);
    assert_eq!(command.0.identity(), identity);
    // The independent registry has accepted dispatch; discard even a queued
    // successful Start reply. The process and its exact pidfd remain reachable.
    drop(pending);
    let encoded = registry.list_json(4096).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&encoded).unwrap(),
        serde_json::json!({
            "processes":[{"pid":identity.pid, "tag":"", "config":expected_config}]
        })
    );
    assert!(matches!(
        registry.list_json(encoded.len() - 1),
        Err(RegistryError::ResourceExhausted)
    ));
    let mut output = registry
        .connectable(identity.pid)
        .unwrap()
        .subscribe(identity)
        .unwrap();
    // This marker is newly written through the original stdin; no old output
    // replay is required, and a short scheduling gap cannot change identity.
    command
        .0
        .write_input(identity, b"connected-marker")
        .await
        .unwrap();
    let mut seen = Vec::new();
    timeout(Duration::from_secs(5), async {
        while !seen.ends_with(b"connected-marker") {
            let OutputEvent::Data(data) = output.next().await.unwrap().unwrap() else {
                panic!("early End");
            };
            seen.extend_from_slice(data.bytes());
        }
    })
    .await
    .unwrap();
    command.0.close_input(identity).unwrap();
    let exit = command.0.wait().await.unwrap();
    assert_eq!(exit.code, 0);
    assert!(command.0.status().completed_at.is_some());
    while let Some(event) = output.next().await {
        if matches!(event.unwrap(), OutputEvent::End(_)) {
            break;
        }
    }
    drop(output);
    assert_eq!(registry.list_json(2).unwrap(), b"{}");
    assert!(matches!(
        registry.live(identity.pid),
        Err(RegistryError::NotFound)
    ));
    assert_eq!(registry.available_slots(), 0);
    let retained = registry.connectable(identity.pid).unwrap();
    let mut terminal = retained.subscribe(identity).unwrap();
    assert!(matches!(terminal.next().await, Some(Ok(OutputEvent::End(value))) if value == exit));
    assert!(terminal.next().await.is_none());
    expire_retention(
        &registry,
        identity,
        command.0.status().completed_at.unwrap(),
    )
    .await;
    registry_capacity(&registry).await;
    assert!(
        matches!(
            fixture.owners.reserve(),
            Err(CommandError::ResourceExhausted)
        ),
        "external handles still own the process record"
    );
    drop(terminal);
    drop(retained);
    drop(command);
    capacity_restored(&fixture.owners).await;
    assert_eq!(
        std::fs::read(fixture.directory.path().join("effect")).unwrap(),
        b"once"
    );
    assert_eq!(fixture.gate_count(), 0);
}

async fn expire_retention(registry: &PidRegistry, identity: CommandIdentity, finished: Instant) {
    tokio::time::pause();
    tokio::time::advance(
        (finished + Duration::from_secs(29)).saturating_duration_since(Instant::now()),
    )
    .await;
    assert!(registry.connectable(identity.pid).is_ok());
    tokio::time::advance(Duration::from_secs(1)).await;
    // Lookup checks the original timestamp even before the timer task runs.
    assert!(matches!(
        registry.connectable(identity.pid),
        Err(RegistryError::NotFound)
    ));
    tokio::time::resume();
}

async fn wait_live(registry: &PidRegistry, pid: u32) -> ManagedCommand {
    timeout(Duration::from_secs(5), async {
        loop {
            match registry.live(pid) {
                Ok(command) => break command,
                Err(RegistryError::NotReady) => tokio::task::yield_now().await,
                Err(error) => panic!("lost registry launch: {error:?}"),
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn original_start_reply_preserves_first_output_and_exec_failure_cannot_become_retained_success()
 {
    for fail in [false, true] {
        let fixture = Fixture::new().await;
        let registry = registry();
        let mut spec = fixture.spec();
        if fail {
            spec.launch.program = fixture
                .directory
                .path()
                .join("missing-program")
                .to_str()
                .unwrap()
                .into();
        }
        let metadata = ProcessConfig {
            cmd: spec.launch.program.clone(),
            args: spec.launch.args.clone(),
            envs: spec.launch.env.clone(),
            cwd: Some(spec.launch.cwd.to_str().unwrap().into()),
        };
        let reservation = registry.reserve(&metadata, None).unwrap();
        let ticket = fixture
            .pending(spec)
            .await
            .register_pid(PidRegistrar::new(store(&fixture)), binding())
            .await
            .unwrap();
        let identity = ticket.identity();
        let _guard = kernel_guard(identity);
        let result = reservation.publish(ticket).unwrap().started().await;
        if fail {
            assert!(matches!(result, Err(CommandError::ExecFailed(2))));
            fixture.assert_no_effect();
        } else {
            let (command, mut stream) = result.unwrap();
            let command = Stop(command);
            let OutputEvent::Data(data) = stream.next().await.unwrap().unwrap() else {
                panic!("missing first output");
            };
            assert_eq!(data.bytes(), b"ready");
            drop(data);
            command.0.close_input(identity).unwrap();
            assert_eq!(command.0.wait().await.unwrap().code, 0);
            assert!(matches!(stream.next().await, Some(Ok(OutputEvent::End(_)))));
            expire_retention(
                &registry,
                identity,
                command.0.status().completed_at.unwrap(),
            )
            .await;
            drop(stream);
            drop(command);
        }
        registry_capacity(&registry).await;
        capacity_restored(&fixture.owners).await;
        assert!(matches!(
            registry.connectable(identity.pid),
            Err(RegistryError::NotFound)
        ));
        assert_eq!(fixture.gate_count(), 0);
        assert!(matches!(
            reopen(&fixture).unwrap().claim(identity),
            Err(ClaimError::Reused)
        ));
    }
}

#[tokio::test]
async fn wrong_vm_or_session_registry_cannot_release_a_durably_registered_worker() {
    for wrong_vm in [false, true] {
        let fixture = Fixture::new().await;
        let registry = PidRegistry::new(
            if wrong_vm {
                Binding::new([0x77; 32]).unwrap()
            } else {
                binding()
            },
            if wrong_vm { 1 } else { 2 },
            RegistryLimits {
                records: 1,
                metadata_bytes: 4096,
                retention: Duration::from_secs(30),
            },
        )
        .unwrap();
        let reservation = registry.reserve(&config(&fixture), None).unwrap();
        let registrar = PidRegistrar::new(store(&fixture));
        let ticket = fixture
            .pending(fixture.spec())
            .await
            .register_pid(registrar, binding())
            .await
            .unwrap();
        let identity = ticket.identity();
        let _guard = kernel_guard(identity);
        let mut observer = ticket.observer();
        assert!(matches!(
            reservation.publish(ticket),
            Err(RegistryError::StaleIdentity)
        ));
        assert_eq!(stopped(&mut observer).await, CommandError::Cancelled);
        registry_capacity(&registry).await;
        capacity_restored(&fixture.owners).await;
        fixture.assert_no_effect();
        assert_eq!(fixture.gate_count(), 0);
    }
}
