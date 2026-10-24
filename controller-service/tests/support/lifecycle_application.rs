//! Actual application and `SQLite`; host facts below are controlled test inputs.
#[allow(
    dead_code,
    reason = "shared original fixture serves several executables"
)]
#[path = "internal_application.rs"]
pub mod fixture;
use fixture::{State, TestClock, TestHost, complete_fixture};
use sandbox_controller_core as core;
use sandproof_controller_service::{
    ControllerJournal, LifecycleInboxConfig, NeverCrash,
    application::{ApplicationConfig, ApplicationError, ControllerApplication},
    capture::HostEpochRegistration,
};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity, ContentDigest,
    ExpectedSession, IncarnationId, LifecycleOperation, LifecycleRequest, OperationBinding,
    OperationKey,
};
use std::{
    path::Path,
    sync::{Arc, Mutex, atomic::AtomicI64},
};
const NS: &str = "0123456789abcdef";
// N=8 covers the six native operations of two unload/restore cycles, plus
// two bounded failed attempts, with the last position reserved for Destroy.
// SQL lease: <=30 MiB per sandbox from G0*6; 64 MiB holds both sessions.
// Checkpoint: (67+6+534+932+2)*4096 + policy copies < 16 MiB.
// SQLite: 8*(64+2*16+2*4) MiB + (8*(4096+2000+2)+64)*4096
// is below 1024 MiB. The WAL frame/SHM envelope and two-image scratch give
// the complete online bound below; these are test deployment data only.
pub const CLEANUP_BYTES: u64 = (1024 + 1040 + 4 + 32) * 1024 * 1024;

pub fn limits() -> sandproof_controller_service::AdmissionLimits {
    sandproof_controller_service::AdmissionLimits {
        max_active_global: 64,
        max_active_per_tenant: 64,
        max_disk_bytes_global: CLEANUP_BYTES + 100_000,
        max_disk_bytes_per_tenant: 100_000,
        cleanup_reserve_bytes: CLEANUP_BYTES,
        max_timeout_secs: 600,
        max_lifetime_secs: 600,
        startup_timeout_secs: 60,
    }
}

pub fn config() -> LifecycleInboxConfig {
    let node = fixture::wire::node("host-a");
    LifecycleInboxConfig {
        center: CenterIdentity {
            deployment: node.deployment_id.clone(),
            center: "primary".try_into().unwrap(),
        },
        node,
        host_epoch: Some(HostEpochRegistration {
            version: 1,
            boot_id: "fixture-boot".into(),
            policy: ContentDigest::from_sha256([7; 32]),
        }),
        max_sessions: 2,
        max_operations_per_session: 8,
        max_options_bytes: 1024,
        max_profile_bytes: 4096,
        max_manifest_bytes: 16384,
    }
}
pub fn context() -> AuthenticatedCenterContext {
    let config = config();
    AuthenticatedCenterContext::from_verified_peer_registration(config.center, config.node)
}
pub fn request() -> AcceptedLifecycleIntent {
    AcceptedLifecycleIntent {
        retention: sandproof_protocol::lifecycle::RetentionBinding {
            slot: 1,
            window: 1.try_into().unwrap(),
        },
        center: config().center,
        accepted_version: 1,
        request: LifecycleRequest {
            binding: OperationBinding {
                protocol_version: 3,
                node: config().node,
                session: "session-a".try_into().unwrap(),
                key: OperationKey {
                    deployment: config().node.deployment_id,
                    owner: "account-alpha".try_into().unwrap(),
                    window: 1.try_into().unwrap(),
                    sequence: 1.try_into().unwrap(),
                },
                expected: ExpectedSession::Absent,
            },
            operation: LifecycleOperation::Create {
                create: fixture::request().create,
                target_incarnation: IncarnationId::try_from(101).unwrap(),
            },
        },
    }
}
pub fn open(path: &Path) -> ControllerJournal {
    let reserve = CLEANUP_BYTES + 1000;
    let maintenance = maintenance();
    ControllerJournal::open_complete(
        path,
        NS,
        complete_fixture::host(path, NS, 64, 2 * 1024 * 1024, reserve),
        maintenance,
        0,
    )
    .unwrap()
}

// Explicit component deployment for bounded operation history plus permanent
// per-position retirement stamps. Production capacity checks are unchanged.
pub fn maintenance() -> sandproof_controller_service::MaintenanceConfig {
    let mut limits = complete_fixture::maintenance();
    limits.max_operations_per_sandbox = 8;
    limits.max_checkpoint_bytes = 16 * 1024 * 1024;
    let cap = limits.capacity.as_mut().unwrap();
    cap.max_sql_bytes = 64 * 1024 * 1024;
    cap.max_sql_value_bytes = 1024 * 1024;
    cap.max_database_bytes = 1024 * 1024 * 1024;
    cap.max_wal_bytes = 1040 * 1024 * 1024;
    cap.max_shm_bytes = 4 * 1024 * 1024;
    cap.checkpoint_scratch_bytes = 32 * 1024 * 1024;
    cap.max_online_storage_bytes = CLEANUP_BYTES;
    limits
}
pub fn source_limits() -> sandproof_controller_service::capture::SourceObservationConfig {
    sandproof_controller_service::capture::SourceObservationConfig {
        max_work_bytes: 32_768,
        max_configuration_bytes: 1024,
        max_executable_bytes: 1024 * 1024,
        max_kernel_bytes: 1024 * 1024,
        max_disk_bytes: 1024 * 1024,
        max_material_bytes: 64 * 1024 * 1024 * 1024,
    }
}

pub fn application(
    path: &Path,
    state: Arc<Mutex<State>>,
    provision: bool,
) -> ControllerApplication<TestHost> {
    application_with_configuration(path, state, provision, config())
}

pub fn application_with_configuration(
    path: &Path,
    state: Arc<Mutex<State>>,
    provision: bool,
    inbox: LifecycleInboxConfig,
) -> ControllerApplication<TestHost> {
    let reserve = CLEANUP_BYTES + 1000;
    application_with_host_configuration(
        path,
        state,
        provision,
        inbox,
        complete_fixture::host(path, NS, 64, 2 * 1024 * 1024, reserve),
    )
}

/// A component can select its original protected physical pool registration.
/// All inbox, source and application checks are the same as the default fixture.
pub fn application_with_host_configuration(
    path: &Path,
    state: Arc<Mutex<State>>,
    provision: bool,
    inbox: LifecycleInboxConfig,
    host: sandproof_controller_service::BoundedHostConfig,
) -> ControllerApplication<TestHost> {
    application_with_active_limit(path, state, provision, inbox, host, 64)
}

pub fn application_with_active_limit(
    path: &Path,
    state: Arc<Mutex<State>>,
    provision: bool,
    inbox: LifecycleInboxConfig,
    host: sandproof_controller_service::BoundedHostConfig,
    active_limit: u64,
) -> ControllerApplication<TestHost> {
    let mut journal = ControllerJournal::open_complete(path, NS, host, maintenance(), 0).unwrap();
    if provision {
        journal
            .initialize_lifecycle_inbox(inbox, source_limits(), &mut NeverCrash)
            .unwrap();
    }
    let reserve = CLEANUP_BYTES + 1000;
    let app_config = ApplicationConfig {
        public_ipv4_enabled: false,
        limits: sandproof_controller_service::AdmissionLimits {
            max_active_global: active_limit,
            max_active_per_tenant: active_limit,
            max_disk_bytes_global: reserve + 2 * 1024 * 1024,
            max_disk_bytes_per_tenant: 2 * 1024 * 1024,
            cleanup_reserve_bytes: reserve,
            max_timeout_secs: 300,
            max_lifetime_secs: 300,
            startup_timeout_secs: 300,
        },
        disk_bytes: 4096,
        file_policy_digest: core::PolicyDigest(1),
        cpu_count: 1,
        memory_mb: 128,
        disk_size_mb: 1,
        envd_version: "fixed-envd".into(),
        domain: "example.invalid".into(),
    };
    ControllerApplication::new(journal, TestHost(state), app_config)
        .with_clock(Arc::new(TestClock(AtomicI64::new(10))))
}
pub fn advance(
    app: &ControllerApplication<TestHost>,
) -> Result<sandproof_controller_service::LifecycleWorkState, ApplicationError> {
    app.advance_lifecycle_create_with_verification(
        &request(),
        &context(),
        Some(fixture::template()),
        fixture::compute(),
        None,
    )
}
