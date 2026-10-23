//! Test assembly only. Current full messages use the original application.
#[path = "../../../controller-service/tests/support/lifecycle_application.rs"]
pub mod current;
pub use current::{config, context, request};
pub use current::fixture::{State, TestClock, TestHost, complete_fixture, compute, template};
use sandproof_controller_service::{
    InternalTemplate,
    application::{ApplicationError, ControllerApplication},
};
use sandproof_host_service::{DriverConfig, HostLifecycle, TemplateSource};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, ExpectedSession, LifecycleAccessIdentity, LifecycleOperation,
};
use std::{path::Path, sync::{Arc, Mutex}, time::Duration};

pub fn state() -> State {
    State {
        epoch: current::fixture::TestEpoch::Matching,
        source: Some(sandproof_controller_service::capture::SourceProcessIdentity {
            pid: 1234,
            start_time_ticks: 5678,
            boot_id: "fixture-boot".into(),
        }),
        ..State::default()
    }
}

pub fn application(
    path: &Path,
    state: Arc<Mutex<State>>,
    clock: Arc<TestClock>,
    provision: bool,
) -> Arc<ControllerApplication<TestHost>> {
    let app = current::application(path, state, provision).with_clock(clock);
    // Explicit controlled provider check after every open; a stored old epoch
    // alone is not permission to run pending startup work after a restart.
    app.establish_lifecycle_host_epoch().unwrap();
    Arc::new(app)
}

pub fn audit(path: &Path) -> sandproof_controller_service::LifecycleAudit {
    let reserve = complete_fixture::CLEANUP_BYTES + 1000;
    sandproof_controller_service::ControllerJournal::audit_lifecycle_complete(
        path,
        "0123456789abcdef",
        &complete_fixture::host(path, "0123456789abcdef", 64, 2 * 1024 * 1024, reserve),
        &current::maintenance(),
        &config(),
    ).unwrap()
}

pub fn application_with_host_config(
    path: &Path,
    state: Arc<Mutex<State>>,
    clock: Arc<TestClock>,
    provision: bool,
    host: sandproof_controller_service::BoundedHostConfig,
) -> Arc<ControllerApplication<TestHost>> {
    let app = current::application_with_host_configuration(path, state, provision, config(), host)
        .with_clock(clock);
    app.establish_lifecycle_host_epoch().unwrap();
    Arc::new(app)
}

pub fn create_mut(request: &mut AcceptedLifecycleIntent) -> &mut sandproof_protocol::internal::lifecycle::CreateBinding {
    let LifecycleOperation::Create { create, .. } = &mut request.request.operation else {
        panic!("expected original Create fixture")
    };
    create
}

pub fn destroy(original: &AcceptedLifecycleIntent) -> AcceptedLifecycleIntent {
    let mut next = original.clone();
    next.accepted_version += 1;
    next.request.binding.expected = ExpectedSession::Version(original.accepted_version);
    next.request.binding.key.sequence = (original.request.binding.key.sequence.get() + 1).try_into().unwrap();
    next.request.operation = LifecycleOperation::Destroy {};
    next
}

pub fn renew(original: &AcceptedLifecycleIntent, end_at_ms: i64) -> AcceptedLifecycleIntent {
    let mut next = destroy(original);
    next.request.operation = LifecycleOperation::Renew {
        incarnation: 101.try_into().unwrap(),
        end_at_ms,
        extend_only: false,
    };
    next
}

pub fn access(intent: &AcceptedLifecycleIntent) -> LifecycleAccessIdentity {
    LifecycleAccessIdentity {
        protocol_version: intent.request.binding.protocol_version,
        center: intent.center.clone(),
        node: intent.request.binding.node.clone(),
        session: intent.request.binding.session.clone(),
        operation: intent.request.binding.key.clone(),
        accepted_version: intent.accepted_version,
        incarnation: 101.try_into().unwrap(),
    }
}

pub struct Templates;
impl TemplateSource for Templates {
    fn bound(&self, binding: &sandproof_protocol::internal::lifecycle::CreateBinding) -> Result<InternalTemplate, ApplicationError> {
        let candidate = template();
        if candidate.revision != binding.template_revision || candidate.digest != binding.template_digest {
            return Err(ApplicationError::NotFound);
        }
        Ok(candidate)
    }
}

pub fn driver_config() -> DriverConfig {
    DriverConfig { node: config().node, compute: compute(), interval: Duration::from_millis(100) }
}

pub async fn drive(host: &HostLifecycle<TestHost>) {
    host.drive_once(driver_config(), Arc::new(Templates)).await.unwrap();
}
