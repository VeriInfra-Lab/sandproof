//! The storage/host substitutes inject faults around the SAME pure center
//! transitions. These tests do not claim Redis, TLS or KVM acceptance.
use super::*;
use sandproof_api::{ControllerPort, CreateRequest, ListQuery, PortError};
use sandproof_center_core::{
    AvailableTemplate, Bounds, CenterState, NodeMode, NodeRegistration, TenantBudget,
};
use sandproof_center_store::{StoreError, authority::AuthorityError};
use sandproof_controller_service::{
    application::{Clock, ConnectCommand, CreateCommand},
    templates::ResourceShape,
};
use sandproof_protocol::internal::{
    NodeIdentity,
    lifecycle::{
        CreateBinding, Intent, LifecycleFact, LifecycleRequest, LifecycleResponse, Resources,
    },
};
use sandproof_protocol::{SandboxId, TenantId};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Clone)]
struct TestClock(Arc<AtomicU64>);
impl Clock for TestClock {
    fn now_ms(&self) -> Result<i64, PortError> {
        i64::try_from(self.0.load(Ordering::SeqCst)).map_err(|_| PortError::Busy)
    }
}
#[derive(Clone)]
struct Templates;
fn template() -> PublicTemplate {
    PublicTemplate::from_binding(
        sandproof_controller_service::templates::TemplateBinding {
            template_id: "base".into(),
            revision: "r1".into(),
            owner: None,
            kernel: sandproof_controller_service::templates::TemplateArtifact {
                path: "/protected/kernel".into(),
                sha256: "1".repeat(64),
                size_bytes: 4096,
            },
            rootfs: sandproof_controller_service::templates::TemplateArtifact {
                path: "/protected/rootfs".into(),
                sha256: "2".repeat(64),
                size_bytes: 1024 * 1024 * 1024,
            },
            resources: ResourceShape {
                cpu_count: 1,
                memory_mib: 128,
                host_memory_bytes: 129 * 1024 * 1024,
                disk_bytes: 1024 * 1024 * 1024,
            },
            envd_version: "0.9.0".into(),
            startup: None,
        },
        "base-r1".try_into().unwrap(),
    )
    .unwrap()
}
impl TemplateProvider for Templates {
    fn resolve(&self, _: &TenantId, name: &str) -> Result<PublicTemplate, PortError> {
        if name == "base" || name == "alias" {
            Ok(template())
        } else {
            Err(PortError::NotFound)
        }
    }
    fn bound(
        &self,
        revision: &sandproof_protocol::ResourceId,
        digest: &[u8; 32],
    ) -> Result<PublicTemplate, PortError> {
        let value = template();
        if value.revision == *revision && value.digest == *digest {
            Ok(value)
        } else {
            Err(PortError::Busy)
        }
    }
    fn publish(&self, _: &str, _: &str) -> Result<sandproof_api::PublicationView, PortError> {
        Err(PortError::NotFound)
    }
    fn publication(&self, _: &str, _: &str) -> Result<sandproof_api::PublicationView, PortError> {
        Err(PortError::NotFound)
    }
    fn get(&self, _: &str, _: &str) -> Result<sandproof_api::TemplateView, PortError> {
        Err(PortError::NotFound)
    }
    fn list(
        &self,
        _: &str,
        _: usize,
        _: Option<&str>,
    ) -> Result<sandproof_api::TemplatePage, PortError> {
        Err(PortError::NotFound)
    }
    fn set_alias(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> Result<sandproof_api::TemplateView, PortError> {
        Err(PortError::NotFound)
    }
    fn delete_alias(&self, _: &str, _: &str) -> Result<(), PortError> {
        Err(PortError::NotFound)
    }
}
fn resources(count: u64) -> Resources {
    let shape = template().resources;
    Resources {
        instances: count,
        cpu: count,
        memory_bytes: shape.host_memory_bytes * count,
        disk_bytes: shape.disk_bytes * count,
    }
}
fn state() -> CenterState {
    let identity = NodeIdentity {
        deployment_id: "deployment".try_into().unwrap(),
        node_id: "host".try_into().unwrap(),
        node_generation: 1.try_into().unwrap(),
    };
    CenterState::new(
        identity.deployment_id.clone(),
        Bounds {
            max_nodes: 1,
            max_templates_per_node: 1,
            max_sandboxes: 8,
            operations_per_sandbox: 8,
            max_options_bytes: 4096,
            platform: resources(8),
        },
        vec![NodeRegistration {
            identity,
            limit: resources(8),
            mode: NodeMode::Accepting,
            healthy_until_ms: u64::MAX,
            free: resources(8),
            templates: vec![AvailableTemplate {
                revision: template().revision,
                digest: template().digest,
            }],
            protocol_version: 1,
        }],
        vec![
            TenantBudget {
                tenant: "1".try_into().unwrap(),
                limit: resources(8),
            },
            TenantBudget {
                tenant: "2".try_into().unwrap(),
                limit: resources(8),
            },
        ],
    )
    .unwrap()
}
struct MemoryAuthority {
    state: CenterState,
    unreadable: Arc<AtomicBool>,
    unknown_create: Arc<AtomicBool>,
}
impl AuthorityPort for MemoryAuthority {
    fn snapshot(&mut self) -> Result<CenterState, AuthorityError> {
        if self.unreadable.load(Ordering::SeqCst) {
            return Err(AuthorityError::RecoveryRequired);
        }
        Ok(self.state.clone())
    }
    fn create(
        &mut self,
        tenant: &TenantId,
        binding: CreateBinding,
        now: u64,
    ) -> Result<LifecycleRequest, AuthorityError> {
        self.snapshot()?;
        let (state, request) = self.state.create(tenant, binding, now)?;
        self.state = state;
        if self.unknown_create.load(Ordering::SeqCst) {
            self.unreadable.store(true, Ordering::SeqCst);
            return Err(AuthorityError::Store(StoreError::OutcomeUnknown));
        }
        Ok(request)
    }
    fn renew(
        &mut self,
        tenant: &TenantId,
        sandbox: &SandboxId,
        end: u64,
        now: u64,
    ) -> Result<LifecycleRequest, AuthorityError> {
        self.snapshot()?;
        let (state, request) = self.state.renew(tenant, sandbox, end, now)?;
        self.state = state;
        Ok(request)
    }
    fn destroy(
        &mut self,
        tenant: &TenantId,
        sandbox: &SandboxId,
    ) -> Result<LifecycleRequest, AuthorityError> {
        self.snapshot()?;
        let (state, request) = self.state.destroy(tenant, sandbox)?;
        self.state = state;
        Ok(request)
    }
    fn apply_response(&mut self, response: &LifecycleResponse) -> Result<(), AuthorityError> {
        self.snapshot()?;
        self.state = self.state.apply_response(response)?;
        Ok(())
    }
    fn compact_confirmed(&mut self, response: &LifecycleResponse) -> Result<(), AuthorityError> {
        self.snapshot()?;
        self.state = self.state.compact_confirmed(response)?;
        Ok(())
    }
    fn recover(&mut self) -> Result<(), AuthorityError> {
        self.unreadable.store(false, Ordering::SeqCst);
        Ok(())
    }
}
#[derive(Clone)]
struct Host {
    calls: Arc<AtomicU64>,
    observations: Arc<AtomicU64>,
    unknown: Arc<AtomicBool>,
    unknown_confirmation: Arc<AtomicBool>,
    recorded: Arc<AtomicBool>,
    last: Arc<Mutex<Option<LifecycleRequest>>>,
    responses: Arc<Mutex<std::collections::BTreeMap<String, LifecycleResponse>>>,
    fence_after_send: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    gate: Arc<Mutex<Option<NetworkGate>>>,
}
struct NetworkGate {
    entered: std::sync::mpsc::SyncSender<()>,
    release: std::sync::mpsc::Receiver<()>,
}
impl Default for Host {
    fn default() -> Self {
        Self {
            calls: Arc::new(AtomicU64::new(0)),
            observations: Arc::new(AtomicU64::new(0)),
            unknown: Arc::new(AtomicBool::new(false)),
            unknown_confirmation: Arc::new(AtomicBool::new(false)),
            recorded: Arc::new(AtomicBool::new(false)),
            last: Arc::new(Mutex::new(None)),
            responses: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
            fence_after_send: Arc::new(Mutex::new(None)),
            gate: Arc::new(Mutex::new(None)),
        }
    }
}
impl HostPort for Host {
    fn confirm(
        &self,
        result: &LifecycleResponse,
        _: Duration,
    ) -> Result<LifecycleResponse, PortError> {
        if self.unknown_confirmation.load(Ordering::SeqCst) {
            return Err(PortError::OutcomeUnknown);
        }
        if self
            .responses
            .lock()
            .unwrap()
            .get(result.identity.operation_id.as_str())
            != Some(result)
        {
            return Err(PortError::Conflict);
        }
        Ok(result.clone())
    }
    fn send(
        &self,
        request: &LifecycleRequest,
        _: Duration,
    ) -> Result<LifecycleResponse, PortError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last.lock().unwrap() = Some(request.clone());
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.entered.send(()).unwrap();
            gate.release.recv().unwrap();
        }
        if self.unknown.load(Ordering::SeqCst) {
            return Err(PortError::Busy);
        }
        let fact = if self.recorded.load(Ordering::SeqCst) {
            LifecycleFact::Recorded
        } else {
            match request.intent {
                Intent::Create => LifecycleFact::Ready {
                    started_at_ms: 1000,
                    end_at_ms: 1000 + request.create.timeout_ms,
                    max_end_at_ms: 1000 + request.create.max_lifetime_ms,
                },
                Intent::Renew { end_at_ms } => LifecycleFact::Renewed { end_at_ms },
                Intent::Destroy => LifecycleFact::Cleaned,
            }
        };
        let mut responses = self.responses.lock().unwrap();
        let key = request.identity.operation_id.as_str().to_owned();
        if let Some(previous) = responses.get(&key)
            && previous.fact == fact
        {
            return Ok(previous.clone());
        }
        let response = LifecycleResponse {
            identity: request.identity.clone(),
            observation_version: (self.observations.fetch_add(1, Ordering::SeqCst) + 1)
                .try_into()
                .unwrap(),
            fact,
        };
        responses.insert(key, response.clone());
        if let Some(fence) = self.fence_after_send.lock().unwrap().take() {
            fence.store(true, Ordering::SeqCst);
        }
        Ok(response)
    }
}
fn config() -> CenterConfig {
    CenterConfig {
        domain: "example.invalid".into(),
        max_timeout_secs: 300,
        max_lifetime_secs: 600,
        startup_timeout_secs: 180,
        reply_timeout: Duration::from_millis(20),
        poll_interval: Duration::from_millis(1),
        public_ipv4_enabled: false,
    }
}

#[test]
fn durable_confirmation_keeps_repeated_renewals_bounded_and_lost_ack_is_retried() {
    let f = fixture();
    let session = f.app.create("1", command()).unwrap();
    for _ in 0..20 {
        f.app.set_timeout("1", &session.sandbox_id, 200).unwrap();
        assert_eq!(f.app.snapshot().unwrap().sandboxes()[0].operations.len(), 2);
    }
    f.host.unknown_confirmation.store(true, Ordering::SeqCst);
    f.app.set_timeout("1", &session.sandbox_id, 100).unwrap();
    let retained = f.app.snapshot().unwrap();
    assert_eq!(retained.sandboxes()[0].operations.len(), 3);
    assert_eq!(retained.sandboxes()[0].end_at_ms, Some(101_000));
    assert!(retained.pending().is_empty());
    f.host.unknown_confirmation.store(false, Ordering::SeqCst);
    f.app.reconcile_once().unwrap();
    let compacted = f.app.snapshot().unwrap();
    assert_eq!(compacted.sandboxes()[0].operations.len(), 2);
    assert_eq!(compacted.sandboxes()[0].end_at_ms, Some(101_000));
    assert!(f.app.resolve_data_route(&session.envd_access_token).is_ok());
    f.app.kill("1", &session.sandbox_id).unwrap();
}
type App = CenterApplication<MemoryAuthority, Host, Templates>;
struct Fixture {
    app: Arc<App>,
    clock: TestClock,
    host: Host,
    unreadable: Arc<AtomicBool>,
    unknown_create: Arc<AtomicBool>,
}
fn fixture() -> Fixture {
    let unreadable = Arc::new(AtomicBool::new(false));
    let unknown_create = Arc::new(AtomicBool::new(false));
    let host = Host::default();
    let clock = TestClock(Arc::new(AtomicU64::new(1000)));
    let app = Arc::new(
        CenterApplication::new(
            MemoryAuthority {
                state: state(),
                unreadable: Arc::clone(&unreadable),
                unknown_create: Arc::clone(&unknown_create),
            },
            host.clone(),
            Templates,
            config(),
            &[13; 32],
        )
        .unwrap()
        .with_clock(Arc::new(clock.clone())),
    );
    Fixture {
        app,
        clock,
        host,
        unreadable,
        unknown_create,
    }
}
fn command() -> CreateCommand {
    CreateCommand {
        template_id: "base".into(),
        timeout: None,
        allow_internet_access: Some(false),
        network: None,
        metadata: Some(std::collections::BTreeMap::from([(
            "group".into(),
            "blue".into(),
        )])),
        env_vars: Some(std::collections::BTreeMap::from([(
            "VALUE".into(),
            "x".into(),
        )])),
    }
}

#[test]
fn public_create_get_list_reuse_original_options_and_sdk_pagination() {
    let f = fixture();
    let mut request = command();
    request.template_id = "alias".into();
    let session = f.app.create("1", request).unwrap();
    assert_eq!(session.template_id, "base");
    assert_eq!(session.sandbox_id, "sandbox-1");
    let wire = f.host.last.lock().unwrap().clone().unwrap();
    let options: sandproof_controller_service::CreateOptions =
        serde_json::from_slice(&wire.create.options).unwrap();
    assert!(
        options.template.is_none(),
        "local host paths are never center input"
    );
    assert_eq!(options.env_vars["VALUE"], "x");
    let port = CenterPort::new(Arc::clone(&f.app));
    let detail = port.get("1", &session.sandbox_id).unwrap();
    assert_eq!(detail.info.metadata["group"], "blue");
    assert_eq!(detail.envd_access_token, session.envd_access_token);
    let page = port
        .list(
            "1",
            ListQuery {
                metadata: Some("group=blue".into()),
                ..ListQuery::default()
            },
        )
        .unwrap();
    assert_eq!(page.total_running, Some(1));
    assert_eq!(page.items[0].sandbox_id, session.sandbox_id);
    assert!(
        port.list("2", ListQuery::default())
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(port.get("2", &session.sandbox_id), Err(PortError::NotFound));
}

#[test]
fn connect_defaults_extend_only_after_host_confirmation_and_keep_public_token() {
    let f = fixture();
    let mut request = command();
    request.timeout = Some(10);
    let original = f.app.create("1", request).unwrap();
    f.clock.0.store(2000, Ordering::SeqCst);
    let renewed = f
        .app
        .connect("1", &original.sandbox_id, ConnectCommand::default())
        .unwrap();
    assert_eq!(renewed.envd_access_token, original.envd_access_token);
    assert_eq!(
        f.app.get("1", &original.sandbox_id).unwrap().info.end_at_ms,
        302_000
    );
    f.app
        .connect(
            "1",
            &original.sandbox_id,
            ConnectCommand { timeout: Some(1) },
        )
        .unwrap();
    assert_eq!(
        f.app.get("1", &original.sandbox_id).unwrap().info.end_at_ms,
        302_000
    );
    assert_eq!(
        f.app
            .resolve_data_route(&original.envd_access_token)
            .unwrap()
            .request
            .identity
            .intent_version
            .get(),
        3
    );
}

#[test]
fn set_timeout_can_shorten_and_large_seconds_cap_without_overflow() {
    let f = fixture();
    let session = f.app.create("1", command()).unwrap();
    f.app.set_timeout("1", &session.sandbox_id, 2).unwrap();
    assert_eq!(
        f.app.get("1", &session.sandbox_id).unwrap().info.end_at_ms,
        3000
    );
    f.app
        .set_timeout("1", &session.sandbox_id, i64::MAX)
        .unwrap();
    assert_eq!(
        f.app.get("1", &session.sandbox_id).unwrap().info.end_at_ms,
        601_000
    );
}

#[test]
fn nonpositive_expiry_closes_global_routes_but_holds_quota_until_exact_cleanup() {
    for seconds in [0, -1, i64::MIN] {
        let f = fixture();
        let session = f.app.create("1", command()).unwrap();
        f.app
            .set_timeout("1", &session.sandbox_id, seconds)
            .unwrap();
        assert_eq!(
            f.app.resolve_data_route(&session.envd_access_token),
            Err(PortError::NotFound)
        );
        assert_eq!(
            f.app.snapshot().unwrap().reserved(None, None),
            Some(resources(1))
        );
        assert_eq!(
            f.host.calls.load(Ordering::SeqCst),
            1,
            "expiry is not a fake synchronous stop"
        );
        f.app.reconcile_once().unwrap();
        assert_eq!(
            f.app.snapshot().unwrap().reserved(None, None),
            Some(Resources::ZERO)
        );
    }
}

#[test]
fn host_transport_failure_after_admission_is_unknown_and_retains_original_identity() {
    let f = fixture();
    f.host.unknown.store(true, Ordering::SeqCst);
    assert_eq!(f.app.create("1", command()), Err(PortError::OutcomeUnknown));
    let original = f.app.snapshot().unwrap().pending()[0].clone();
    assert_eq!(original.identity.sandbox_id.as_str(), "sandbox-1");
    f.host.unknown.store(false, Ordering::SeqCst);
    f.app.reconcile_once().unwrap();
    assert_eq!(f.host.last.lock().unwrap().as_ref().unwrap(), &original);
    assert_eq!(f.app.snapshot().unwrap().sandboxes().len(), 1);
}

#[test]
fn unknown_redis_commit_prevents_dispatch_and_cached_read_authorization() {
    let f = fixture();
    f.unknown_create.store(true, Ordering::SeqCst);
    assert_eq!(f.app.create("1", command()), Err(PortError::OutcomeUnknown));
    assert_eq!(f.host.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.app.snapshot(), Err(PortError::OutcomeUnknown));
    f.unknown_create.store(false, Ordering::SeqCst);
    f.app.recover_authority().unwrap();
    f.app.reconcile_once().unwrap();
    assert_eq!(f.app.snapshot().unwrap().sandboxes().len(), 1);
    assert_eq!(
        f.host
            .last
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .identity
            .sandbox_id
            .as_str(),
        "sandbox-1"
    );
}

#[test]
fn fresh_authority_is_required_for_every_token_and_query() {
    let f = fixture();
    let session = f.app.create("1", command()).unwrap();
    assert!(f.app.resolve_data_route(&session.envd_access_token).is_ok());
    f.unreadable.store(true, Ordering::SeqCst);
    assert_eq!(
        f.app.resolve_data_route(&session.envd_access_token),
        Err(PortError::OutcomeUnknown)
    );
    assert_eq!(
        f.app.get("1", &session.sandbox_id),
        Err(PortError::OutcomeUnknown)
    );
    assert_eq!(
        f.app.list(
            "1",
            sandproof_controller_service::application::ListSelection::default()
        ),
        Err(PortError::OutcomeUnknown)
    );
}

#[test]
fn public_tokens_are_instance_bound_and_do_not_route_after_kill_or_expiry() {
    let f = fixture();
    let first = f.app.create("1", command()).unwrap();
    let second = f.app.create("2", command()).unwrap();
    assert_ne!(first.envd_access_token, second.envd_access_token);
    assert_eq!(
        f.app
            .resolve_data_route(&first.envd_access_token)
            .unwrap()
            .request
            .identity
            .tenant_id
            .as_str(),
        "1"
    );
    let mut bad = first.envd_access_token.clone();
    bad.replace_range(..1, if bad.starts_with('a') { "b" } else { "a" });
    assert_eq!(f.app.resolve_data_route(&bad), Err(PortError::NotFound));
    f.app.kill("1", &first.sandbox_id).unwrap();
    assert_eq!(
        f.app.resolve_data_route(&first.envd_access_token),
        Err(PortError::NotFound)
    );
    assert!(f.app.resolve_data_route(&second.envd_access_token).is_ok());
    f.clock.0.store(301_000, Ordering::SeqCst);
    assert_eq!(
        f.app.resolve_data_route(&second.envd_access_token),
        Err(PortError::NotFound)
    );
}

#[test]
fn malformed_options_and_lifetime_refusals_do_not_allocate_or_contact_host() {
    let f = fixture();
    for timeout in [Some(0), Some(-1), Some(301)] {
        let mut input = command();
        input.timeout = timeout;
        assert_eq!(f.app.create("1", input), Err(PortError::Invalid));
    }
    let mut input = command();
    input
        .env_vars
        .as_mut()
        .unwrap()
        .insert("A=B".into(), "x".into());
    assert_eq!(f.app.create("1", input), Err(PortError::Invalid));
    let mut input = command();
    input
        .metadata
        .as_mut()
        .unwrap()
        .insert("_sandproof_internal".into(), "x".into());
    assert_eq!(f.app.create("1", input), Err(PortError::Invalid));
    let mut input = command();
    input.template_id = "other".into();
    assert_eq!(f.app.create("1", input), Err(PortError::NotFound));
    assert_eq!(f.host.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.app.create("1", command()).unwrap().sandbox_id,
        "sandbox-1"
    );
}

#[test]
fn recorded_receipt_timeout_is_not_success_and_original_pending_survives() {
    let f = fixture();
    f.host.recorded.store(true, Ordering::SeqCst);
    assert_eq!(f.app.create("1", command()), Err(PortError::OutcomeUnknown));
    assert_eq!(f.app.snapshot().unwrap().pending().len(), 1);
    assert_eq!(f.app.get("1", "sandbox-1"), Err(PortError::NotFound));
}

#[test]
fn public_adapter_rejects_invalid_list_before_authority_access() {
    let f = fixture();
    f.unreadable.store(true, Ordering::SeqCst);
    let port = CenterPort::new(Arc::clone(&f.app));
    assert_eq!(
        port.list(
            "1",
            ListQuery {
                limit: Some(0),
                ..ListQuery::default()
            }
        ),
        Err(PortError::Invalid)
    );
    assert_eq!(
        port.create(
            "1",
            CreateRequest {
                template_id: "base".into(),
                timeout: Some(-1),
                allow_internet_access: Some(false),
                network: None,
                metadata: None,
                env_vars: None
            }
        ),
        Err(PortError::Invalid)
    );
}

#[test]
fn startup_timeout_creates_destroy_intent_without_guessing_absence() {
    let f = fixture();
    f.host.unknown.store(true, Ordering::SeqCst);
    assert_eq!(f.app.create("1", command()), Err(PortError::OutcomeUnknown));
    let original = f.app.snapshot().unwrap().pending()[0].clone();
    f.clock
        .0
        .store(original.create.startup_deadline_ms, Ordering::SeqCst);
    assert_eq!(f.app.reconcile_once(), Ok(0));
    let terminating = f.app.snapshot().unwrap().pending()[0].clone();
    assert_eq!(terminating.intent, Intent::Destroy);
    assert_eq!(
        terminating.identity.sandbox_id,
        original.identity.sandbox_id
    );
    assert_eq!(terminating.create, original.create);
    assert_eq!(
        f.app.snapshot().unwrap().reserved(None, None),
        Some(resources(1))
    );
    f.host.unknown.store(false, Ordering::SeqCst);
    f.app.reconcile_once().unwrap();
    assert_eq!(
        f.app.snapshot().unwrap().reserved(None, None),
        Some(Resources::ZERO)
    );
}

#[test]
fn background_shared_storage_failure_is_reported_not_counted_as_host_unknown() {
    let f = fixture();
    f.host.unknown.store(true, Ordering::SeqCst);
    assert_eq!(f.app.create("1", command()), Err(PortError::OutcomeUnknown));
    f.host.unknown.store(false, Ordering::SeqCst);
    *f.host.fence_after_send.lock().unwrap() = Some(Arc::clone(&f.unreadable));
    assert_eq!(f.app.reconcile_once(), Err(PortError::OutcomeUnknown));
    assert_eq!(f.app.snapshot(), Err(PortError::OutcomeUnknown));
}

#[test]
fn slow_network_does_not_hold_center_decision_lock_and_late_ready_cannot_revive_destroy() {
    let f = fixture();
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    *f.host.gate.lock().unwrap() = Some(NetworkGate {
        entered: entered_tx,
        release: release_rx,
    });
    let app = Arc::clone(&f.app);
    let creator = std::thread::spawn(move || app.create("1", command()));
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    // NetworkGate has released its own mutex. A destroy can commit and finish
    // while the prior create request is demonstrably still awaiting a reply.
    f.app.kill("1", "sandbox-1").unwrap();
    release_tx.send(()).unwrap();
    assert_eq!(creator.join().unwrap(), Err(PortError::OutcomeUnknown));
    assert!(f.app.snapshot().unwrap().sandboxes()[0].released);
    assert_eq!(f.app.get("1", "sandbox-1"), Err(PortError::NotFound));
}

#[test]
fn public_template_projection_is_cryptographically_bound_to_original_description() {
    let original = template();
    let mut changed = original.clone();
    changed.envd_version = "unrelated".into();
    assert!(!changed.is_valid());
    let mut changed = original.clone();
    changed.template_id = "fake-base".into();
    assert!(!changed.is_valid());
    assert!(original.is_valid());
    assert_eq!(original.digest(), template().digest());
}

#[test]
fn data_token_is_stable_on_same_deployment_restart_but_not_another_key_or_node_generation() {
    let f = fixture();
    let session = f.app.create("1", command()).unwrap();
    let identity = f
        .app
        .resolve_data_route(&session.envd_access_token)
        .unwrap()
        .request
        .identity;
    let same = crate::token::Tokens::new(&[13; 32]).unwrap();
    assert_eq!(same.issue(&identity).unwrap(), session.envd_access_token);
    let other = crate::token::Tokens::new(&[14; 32]).unwrap();
    assert!(!other.verify(&identity, &session.envd_access_token));
    let mut new_generation = identity.clone();
    new_generation.node.node_generation =
        new_generation.node.node_generation.checked_next().unwrap();
    assert!(!same.verify(&new_generation, &session.envd_access_token));
    assert!(crate::token::Tokens::new(&[13; 31]).is_err());
    for invalid in ["", "xx", &"g".repeat(64), &"0".repeat(66)] {
        assert!(!same.verify(&identity, invalid));
    }
}
