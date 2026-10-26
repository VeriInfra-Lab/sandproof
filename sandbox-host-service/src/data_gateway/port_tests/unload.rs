//! Original application/SQLite authority and gateway ownership types. The cat
//! child is the controlled physical relay provider; no namespace/VM is claimed.
use super::*;
use http_body_util::BodyExt;
use sandproof_controller_service::capture::{
    FullCaptureWorkRequest, RawSourceObservation, SourceProcessIdentity,
};
use sandproof_protocol::{
    WorkAction,
    lifecycle::{
        ContentDigest, ExpectedSession, LifecycleOperation, VmCompatibility, WritableDrive,
    },
};

#[path = "unload/debug_busy.rs"]
mod debug_busy;

fn capture_dispatches(path: &std::path::Path) -> Vec<WorkAction<core::EffectId>> {
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let mut query = connection
        .prepare("SELECT action_json FROM outbox WHERE dispatch_state != 'ready' ORDER BY id")
        .unwrap();
    query
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|row| serde_json::from_slice::<WorkAction<core::EffectId>>(&row.unwrap()).unwrap())
        .filter(|action| {
            matches!(
                action.command.command(),
                core::Command::FreezeVm { .. } | core::Command::SealSnapshot { .. }
            )
        })
        .collect()
}

fn prepare_unload(
    app: &ControllerApplication<local::TestHost>,
    state: &std::sync::Mutex<local::State>,
    create: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    context: &AuthenticatedCenterContext,
) -> sandproof_protocol::lifecycle::AcceptedLifecycleIntent {
    let mut unload = create.clone();
    unload.accepted_version = 2;
    unload.request.binding.expected = ExpectedSession::Version(1);
    unload.request.binding.key.sequence = 2.try_into().unwrap();
    unload.request.operation = LifecycleOperation::Unload {
        source_incarnation: 101.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
    };
    app.record_lifecycle_intent(&unload, context).unwrap();
    let question = app.prepare_lifecycle_source_work(&unload, context).unwrap();
    state.lock().unwrap().source_probe = Some(Box::new(|r| {
        Ok(RawSourceObservation {
            work_binding: r.work_binding,
            resource: r.resource.clone(),
            source: r.source.clone(),
            configuration: br#"{"vcpu_count":1}"#.to_vec(),
            executable_sha256: [11; 32],
            kernel_sha256: [12; 32],
            kernel_bytes: 100,
            disk_bytes: 4096,
        })
    }));
    let source = app
        .observe_lifecycle_source_work(&question.work, context)
        .unwrap();
    state.lock().unwrap().full_proposal = Some(FullCaptureWorkRequest {
        pause: source.work.pause_request().unwrap(),
        compatibility: VmCompatibility {
            snapshot_format: 1,
            tool: ContentDigest::from_sha256([1; 32]),
            cpu: ContentDigest::from_sha256([2; 32]),
            devices: ContentDigest::from_sha256([3; 32]),
            image_configuration: ContentDigest::from_sha256([4; 32]),
            writable_drives: vec![WritableDrive {
                drive_id: "rootfs".try_into().unwrap(),
                jail_relative_path: "rootfs.ext4".into(),
            }],
        },
        state_bytes: 4096,
        memory_bytes: 4096,
        disk_bytes: 4096,
        api_timeout_ms: 1000,
        worker_timeout_ms: 1000,
    });
    unload
}

fn current_identity(
    create: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
) -> LifecycleAccessIdentity {
    LifecycleAccessIdentity {
        protocol_version: sandproof_protocol::lifecycle::PROTOCOL_VERSION,
        center: create.center.clone(),
        node: create.request.binding.node.clone(),
        session: create.request.binding.session.clone(),
        operation: create.request.binding.key.clone(),
        accepted_version: create.accepted_version,
        incarnation: 101.try_into().unwrap(),
    }
}

fn running_state() -> Arc<std::sync::Mutex<local::State>> {
    Arc::new(std::sync::Mutex::new(local::State {
        epoch: local::TestEpoch::Matching,
        source: Some(SourceProcessIdentity {
            pid: 1234,
            start_time_ticks: 5678,
            boot_id: "fixture-boot".into(),
        }),
        ..local::State::default()
    }))
}

#[tokio::test]
async fn original_unload_waits_for_both_gateway_body_and_actual_relay_before_first_freeze() {
    for body_first in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let state = running_state();
        let app = lifecycle::application(&path, Arc::clone(&state), true);
        let create = lifecycle::request();
        let context = lifecycle::context();
        app.record_lifecycle_intent(&create, &context).unwrap();
        app.establish_lifecycle_host_epoch().unwrap();
        lifecycle::advance(&app).unwrap();
        let identity = current_identity(&create);
        let (permit, secret) = app
            .lifecycle_data_authorization(&identity, &context, DataKind::FileWrite)
            .unwrap();
        assert!(!*permit.revocation.borrow());

        // Reuse the actual gateway launch/exit and body owners with this exact
        // application's lease. Only the host launch provider is controlled.
        let port = Arc::new(ControlledPort {
            drain: permit.drain().clone(),
            ..ControlledPort::new(None, None)
        });
        let relay_permit = port
            .data_permit("controlled-test-token", DataKind::Exec)
            .unwrap();
        let (relay_permit, mut child) =
            launch_data_route(Arc::clone(&port), relay_permit, false, request_lease())
                .await
                .unwrap();
        let mut exit = drain::RelayExit::new(relay_permit.drain().clone());
        let (mut read, mut write) = child.take_pipes().unwrap();
        write.write_all(b"accepted-before-unload").await.unwrap();
        let mut bytes = [0; 22];
        tokio::time::timeout(Duration::from_secs(5), read.read_exact(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&bytes, b"accepted-before-unload");
        let body = drain::DrainBody::wrap(
            Response::new(Body::from("accepted write result")),
            permit.drain().clone(),
        );
        drop((port, relay_permit));

        let unload = prepare_unload(&app, &state, &create, &context);
        let advance =
            || app.advance_recorded_lifecycle_unload(&unload, &unload.request.binding.node);
        assert_eq!(advance(), Err(PortError::Busy));
        assert!(*permit.revocation.borrow());
        assert!(app.data_permit(&secret, DataKind::FileWrite).is_err());
        assert!(
            app.lifecycle_data_authorization(&identity, &context, DataKind::FileWrite)
                .is_err()
        );
        assert!(matches!(
            app.spawn_data_relay(&permit, false),
            Err(PortError::NotFound)
        ));
        assert!(capture_dispatches(&path).is_empty());
        drop(permit);
        assert_eq!(advance(), Err(PortError::Busy));
        assert!(capture_dispatches(&path).is_empty());

        let mut body = Some(body);
        if body_first {
            let result = body.take().unwrap().into_body().collect().await.unwrap();
            assert_eq!(result.to_bytes(), "accepted write result");
        } else {
            exit.stop(&mut child).await;
        }
        // A stopped child is not enough while its RelayExit owner remains;
        // finish that owner only in the relay-first branch.
        let mut exit = Some(exit);
        if !body_first {
            drop(exit.take());
        }
        assert_eq!(advance(), Err(PortError::Busy));
        assert!(capture_dispatches(&path).is_empty());

        if body_first {
            exit.as_mut().unwrap().stop(&mut child).await;
            drop(exit.take());
        } else {
            let result = body.take().unwrap().into_body().collect().await.unwrap();
            assert_eq!(result.to_bytes(), "accepted write result");
        }
        drop((read, write, child));
        // The fixture cannot physically Pause a VM. The first Freeze is now
        // durably dispatched and remains unknown, with no Full sent/success.
        assert_eq!(advance(), Ok(false));
        let dispatched = capture_dispatches(&path);
        assert_eq!(dispatched.len(), 1);
        assert!(matches!(
            dispatched[0].command.command(),
            core::Command::FreezeVm { .. }
        ));
        assert!(matches!(
            app.observe_lifecycle_unloaded(&unload, &context)
                .unwrap()
                .outcome,
            sandproof_protocol::lifecycle::UnloadOutcome::Pending
        ));
        assert!(state.lock().unwrap().released_slots.is_empty());
    }
}
