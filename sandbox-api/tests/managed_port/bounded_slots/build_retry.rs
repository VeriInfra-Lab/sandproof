//! Private admission/closure races use the same production mutex and reducer.
use super::*;
use sandproof_api::{BuildSettlementRequest, BuildVmRequest};
use sandproof_controller_service::BuildRetryConfig;
const BUILD: &str = "123456789abcdef0123456789abcdef0";
fn input(window: &str) -> BuildVmRequest {
    BuildVmRequest {
        request_window: Some(window.into()),
        timeout: 300,
        allow_internet_access: false,
    }
}
fn port(path: &std::path::Path, host: BoundedHost) -> ManagedPort<BoundedHost> {
    let mut host_config = complete_fixture::builder_host(path);
    host_config.maintenance.max_host_slots = 1;
    let pool = host_config
        .physical_storage
        .as_mut()
        .unwrap()
        .pools
        .iter_mut()
        .find(|pool| pool.namespace == NAMESPACE)
        .unwrap();
    pool.max_host_slots = 1;
    let journal = ControllerJournal::open_complete(
        path,
        NAMESPACE,
        host_config,
        complete_fixture::maintenance(),
        0,
    )
    .unwrap();
    ManagedPort::new(journal, host, config())
        .with_internal_build_vms(true)
        .with_build_retry_windows(BuildRetryConfig {
            window_duration_ms: 3_600_000,
            max_anchors: 1,
        })
        .unwrap()
}
fn settle(
    port: &ManagedPort<BoundedHost>,
    window: &str,
    sandbox: Option<&str>,
) -> Result<sandproof_api::BuildSettlementView, PortError> {
    port.settle_build_vm(
        "1",
        BUILD,
        BuildSettlementRequest {
            request_window: window.into(),
            sandbox_id: sandbox.map(str::to_owned),
        },
    )
}

#[test]
fn delayed_post_behind_committed_close_barrier_never_admits() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let host = BoundedHost::default();
    let port = Arc::new(port(&path, host.clone()));
    let window = port.issue_build_window("1").unwrap().request_window;
    let ready = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let child_port = port.clone();
    let child_window = window.clone();
    let child_ready = ready.clone();
    let child_release = release.clone();
    let thread = std::thread::spawn(move || {
        child_ready.wait();
        child_release.wait();
        child_port.build_vm("1", BUILD, Some(input(&child_window)))
    });
    ready.wait();
    port.close_build_window("1", &window).unwrap();
    assert!(matches!(
        port.build_vm_in_window("1", BUILD, Some(&window), None),
        Err(PortError::NotFound)
    ));
    settle(&port, &window, None).unwrap();
    release.wait();
    assert!(thread.join().unwrap().is_err());
    assert!(host.0.lock().unwrap().actions.is_empty());
    drop(port);
    let reopened = port_for_reopen(&path, host);
    assert!(reopened.build_vm("1", BUILD, Some(input(&window))).is_err());
}
fn port_for_reopen(path: &std::path::Path, host: BoundedHost) -> ManagedPort<BoundedHost> {
    port(path, host)
}

#[test]
fn accepted_before_close_has_same_identity_and_releases_one_slot_only_after_ack() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let host = BoundedHost::default();
    let port = port(&path, host.clone());
    for iteration in 0..3 {
        let window = port.issue_build_window("1").unwrap().request_window;
        let created = port.build_vm("1", BUILD, Some(input(&window))).unwrap();
        port.close_build_window("1", &window).unwrap();
        assert_eq!(
            port.build_vm("1", BUILD, Some(input(&window)))
                .unwrap()
                .sandbox_id,
            created.sandbox_id
        );
        assert!(settle(&port, &window, Some(&created.sandbox_id)).is_err());
        port.kill("1", &created.sandbox_id).unwrap();
        // The unsettled issued-window anchor pins history and its physical
        // position even after VM cleanup; retirement cannot consume it.
        let retired = port.maintain().unwrap().retired_sandboxes;
        if iteration == 0 {
            assert_eq!(retired, 0);
        }
        assert_eq!(
            port.build_vm("1", BUILD, Some(input(&window)))
                .unwrap()
                .sandbox_id,
            created.sandbox_id
        );
        assert!(
            port.build_vm_in_window("1", BUILD, Some(&window), None)
                .unwrap()
                .cleanup_complete
        );
        assert_eq!(
            host.0.lock().unwrap().row.unwrap().1,
            HostSlotPhase::Occupied
        );
        if iteration == 0 {
            host.0.lock().unwrap().lose_release = true;
            assert!(
                settle(&port, &window, Some(&created.sandbox_id))
                    .unwrap()
                    .settled
            );
        }
        settle(&port, &window, Some(&created.sandbox_id)).unwrap();
        assert_eq!(host.0.lock().unwrap().row.unwrap().1, HostSlotPhase::Free);
        assert!(matches!(
            port.build_vm_in_window("1", BUILD, Some(&window), None),
            Err(PortError::NotFound)
        ));
        assert!(port.build_vm("1", BUILD, Some(input(&window))).is_err());
        settle(&port, &window, Some(&created.sandbox_id)).unwrap();
    }
    let prepares = host
        .0
        .lock()
        .unwrap()
        .actions
        .iter()
        .filter(|a| matches!(a.command.command(), core::Command::PrepareVm { .. }))
        .count();
    assert_eq!(prepares, 3);
}

#[test]
fn unknown_cleanup_keeps_anchor_and_blocks_one_slot_reuse() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let host = BoundedHost::default();
    let port = port(&path, host.clone());
    let window = port.issue_build_window("1").unwrap().request_window;
    let created = port.build_vm("1", BUILD, Some(input(&window))).unwrap();
    port.close_build_window("1", &window).unwrap();
    host.0.lock().unwrap().unknown_cleanup = true;
    let _ = port.kill("1", &created.sandbox_id);
    assert!(
        !port
            .build_vm_in_window("1", BUILD, Some(&window), None)
            .unwrap()
            .cleanup_complete
    );
    assert!(settle(&port, &window, Some(&created.sandbox_id)).is_err());
    assert_eq!(
        port.build_vm("1", BUILD, Some(input(&window)))
            .unwrap()
            .sandbox_id,
        created.sandbox_id
    );
    let next = port.issue_build_window("1").unwrap().request_window;
    assert!(port.build_vm("1", BUILD, Some(input(&next))).is_err());
    assert_eq!(
        host.0.lock().unwrap().row.unwrap().1,
        HostSlotPhase::Occupied
    );
}

#[test]
fn exact_request_tenant_and_namespace_are_checked_after_closed_lookup() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let host = BoundedHost::default();
    let port = port(&path, host);
    let window = port.issue_build_window("1").unwrap().request_window;
    let created = port.build_vm("1", BUILD, Some(input(&window))).unwrap();
    port.close_build_window("1", &window).unwrap();
    let mut changed = input(&window);
    changed.timeout += 1;
    assert!(matches!(
        port.build_vm("1", BUILD, Some(changed)),
        Err(PortError::Invalid)
    ));
    assert!(
        port.build_vm_in_window("2", BUILD, Some(&window), None)
            .is_err()
    );
    let wrong = window.replace(NAMESPACE, "fedcba9876543210");
    assert!(port.close_build_window("1", &wrong).is_err());
    assert_eq!(
        port.build_vm("1", BUILD, Some(input(&window)))
            .unwrap()
            .sandbox_id,
        created.sandbox_id
    );
}
