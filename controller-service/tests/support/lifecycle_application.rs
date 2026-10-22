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
    CheckpointProfile, ControllerJournal, LifecycleInboxConfig, NeverCrash,
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
    let reserve = complete_fixture::CLEANUP_BYTES + 1000;
    let mut maintenance = complete_fixture::maintenance();
    maintenance.checkpoint_profile = CheckpointProfile::FirstUnload;
    ControllerJournal::open_complete(
        path,
        NS,
        complete_fixture::host(path, NS, 64, 2 * 1024 * 1024, reserve),
        maintenance,
        0,
    )
    .unwrap()
}
pub fn application(
    path: &Path,
    state: Arc<Mutex<State>>,
    provision: bool,
) -> ControllerApplication<TestHost> {
    let mut journal = open(path);
    if provision {
        journal
            .initialize_lifecycle_inbox(config(), &mut NeverCrash)
            .unwrap();
    }
    let reserve = complete_fixture::CLEANUP_BYTES + 1000;
    let app_config = ApplicationConfig {
        public_ipv4_enabled: false,
        limits: sandproof_controller_service::AdmissionLimits {
            max_active_global: 64,
            max_active_per_tenant: 64,
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
