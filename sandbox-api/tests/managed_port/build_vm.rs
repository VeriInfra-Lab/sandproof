use super::*;
use sandproof_api::BuildVmRequest;

const BUILD: &str = "123456789abcdef0123456789abcdef0";
fn input() -> BuildVmRequest {
    BuildVmRequest {
        request_window: None,
        timeout: 300,
        allow_internet_access: false,
    }
}

#[test]
fn private_pool_opt_in_reserved_metadata_and_tombstone() {
    let (_directory, disabled) = setup();
    assert!(matches!(
        disabled.build_vm("1", BUILD, Some(input())),
        Err(PortError::NotFound)
    ));
    let mut spoofed = request();
    spoofed.metadata = Some(BTreeMap::from([(
        "_sandproof_build_id".into(),
        BUILD.into(),
    )]));
    assert!(matches!(
        disabled.create("1", spoofed),
        Err(PortError::Invalid)
    ));
    let directory = TempDir::new().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let port = Arc::new(
        ManagedPort::new(
            ControllerJournal::open(directory.path().join("pool.db"), NAMESPACE).unwrap(),
            FakeHost {
                prepare_calls: Some(Arc::clone(&calls)),
                ..FakeHost::default()
            },
            config(),
        )
        .with_internal_build_vms(true),
    );
    let barrier = Arc::new(Barrier::new(3));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let port = Arc::clone(&port);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                port.build_vm("1", BUILD, Some(input())).unwrap().sandbox_id
            })
        })
        .collect();
    barrier.wait();
    let ids: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(ids[0], ids[1]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        port.build_vm("2", BUILD, None),
        Err(PortError::NotFound)
    ));
    assert!(matches!(
        port.build_vm(
            "1",
            BUILD,
            Some(BuildVmRequest {
                request_window: None,
                timeout: 301,
                ..input()
            })
        ),
        Err(PortError::Invalid)
    ));
    port.kill("1", &ids[0]).unwrap();
    let dead = port.build_vm("1", BUILD, Some(input())).unwrap();
    assert!(dead.cleanup_complete);
    assert!(dead.session.is_none());
    assert_eq!(dead.sandbox_id, ids[0]);
    assert_eq!(serde_json::to_value(&dead).unwrap()["sandboxID"], ids[0]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn lost_launch_receipt_and_restart_keep_original_build_identity() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("pool.db");
    let calls = Arc::new(AtomicUsize::new(0));
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        FakeHost {
            prepare: PrepareObservation::LiveUnready,
            prepare_calls: Some(Arc::clone(&calls)),
            ..FakeHost::default()
        },
        config(),
    )
    .with_internal_build_vms(true);
    let first = port.build_vm("1", BUILD, Some(input())).unwrap();
    assert!(first.session.is_none());
    drop(port);
    let reopened = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        FakeHost {
            prepare_calls: Some(Arc::clone(&calls)),
            ..FakeHost::default()
        },
        config(),
    )
    .with_internal_build_vms(true);
    let again = reopened.build_vm("1", BUILD, Some(input())).unwrap();
    assert_eq!(first.sandbox_id, again.sandbox_id);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn destroyed_publication_does_not_release_build_budget_before_vm_reclaim() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("pool.db");
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        FakeHost {
            unknown_cleanup_for: Some(core::SandboxId(1)),
            ..FakeHost::default()
        },
        config(),
    )
    .with_internal_build_vms(true);
    let created = port.build_vm("1", BUILD, Some(input())).unwrap();
    assert!(matches!(
        port.kill("1", &created.sandbox_id),
        Err(PortError::OutcomeUnknown)
    ));
    let pending = port.build_vm("1", BUILD, None).unwrap();
    assert_eq!(pending.state, "Destroyed");
    assert!(!pending.cleanup_complete);
    drop(port);
    let restarted = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    )
    .with_internal_build_vms(true);
    restarted.recover_on_startup().unwrap();
    let final_view = restarted.build_vm("1", BUILD, None).unwrap();
    assert!(final_view.cleanup_complete);
    assert_eq!(final_view.sandbox_id, created.sandbox_id);
}
