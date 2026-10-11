//! The storage/host substitutes inject faults around the SAME pure center
//! transitions. These tests do not claim Redis, TLS or KVM acceptance.
mod session_wait;
#[test]
fn physical_observation_has_bounded_freshness_and_clock_skew() {
    let policy = super::NodeObservationPolicy {
        health_window_ms: 1000,
        max_clock_skew_ms: 10,
    };
    assert!(
        policy
            .validate(std::time::Duration::from_millis(100))
            .is_ok()
    );
    assert_eq!(policy.healthy_until(100, 200, 90), Some(1100));
    assert_eq!(policy.healthy_until(100, 200, 210), Some(1100));
    assert_eq!(policy.healthy_until(100, 200, 89), None);
    assert_eq!(policy.healthy_until(100, 200, 211), None);
    assert_eq!(policy.healthy_until(100, 1100, 1100), None);
    assert_eq!(policy.healthy_until(200, 100, 150), None);
    assert_eq!(policy.healthy_until(u64::MAX - 5, u64::MAX, u64::MAX), None);
    assert!(policy.validate(std::time::Duration::from_secs(1)).is_err());
}
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
    window::{WindowCloseAck, WindowCloseRequest},
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
    fn publish(
        &self,
        _: &dyn crate::CommittedDriver,
        _: &str,
        _: &str,
    ) -> Result<sandproof_api::PublicationView, PortError> {
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
            retention_slots: 8,
            role: sandproof_center_core::NodeRole::Runtime,
            identity,
            limit: resources(8),
            mode: NodeMode::Accepting,
            healthy_until_ms: u64::MAX,
            free: resources(8),
            templates: vec![AvailableTemplate {
                revision: template().revision,
                digest: template().digest,
            }],
            protocol_version: 2,
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
    hidden_sandboxes: Vec<SandboxId>,
    unreadable: Arc<AtomicBool>,
    unknown_create: Arc<AtomicBool>,
    projection_steps: std::collections::VecDeque<ProjectionStep>,
    projection_reads: u64,
}
enum ProjectionStep {
    Health(u64),
    SharedFault,
    CompleteAt(std::time::Instant),
}
fn consume_reply_budget(budget: Duration) {
    let (_sender, receiver) = std::sync::mpsc::sync_channel::<()>(0);
    assert_eq!(
        receiver.recv_timeout(budget),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    );
}
impl AuthorityPort for MemoryAuthority {
    fn snapshot(&mut self) -> Result<CenterState, AuthorityError> {
        if self.unreadable.load(Ordering::SeqCst) {
            return Err(AuthorityError::RecoveryRequired);
        }
        Ok(self.state.clone())
    }
    fn public_snapshot(&mut self) -> Result<PublicSnapshot, AuthorityError> {
        self.projection_reads += 1;
        match self.projection_steps.pop_front() {
            Some(ProjectionStep::Health(healthy_until)) => {
                let node = &self.state.nodes()[0];
                self.state = self
                    .state
                    .refresh_node(&node.identity, healthy_until, node.free)?;
            }
            Some(ProjectionStep::SharedFault) => self.unreadable.store(true, Ordering::SeqCst),
            Some(ProjectionStep::CompleteAt(deadline)) => {
                consume_reply_budget(deadline.saturating_duration_since(std::time::Instant::now()));
            }
            None => {}
        }
        Ok(PublicSnapshot {
            state: self.snapshot()?,
            hidden_sandboxes: self.hidden_sandboxes.clone(),
        })
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
    fn confirm_terminal(&mut self, response: &LifecycleResponse) -> Result<(), AuthorityError> {
        self.snapshot()?;
        self.state = self.state.confirm_terminal(response)?;
        Ok(())
    }
    fn begin_drain(&mut self, node: &NodeIdentity) -> Result<(), AuthorityError> {
        self.snapshot()?;
        self.state = self.state.begin_drain(node)?;
        Ok(())
    }
    fn begin_close(
        &mut self,
        terminal: &LifecycleResponse,
    ) -> Result<WindowCloseRequest, AuthorityError> {
        self.snapshot()?;
        let (state, request) = self.state.begin_close(terminal)?;
        self.state = state;
        Ok(request)
    }
    fn close_batch(&mut self) -> Result<Vec<WindowCloseRequest>, AuthorityError> {
        Ok(self.snapshot()?.pending_closes())
    }
    fn apply_close_ack(&mut self, ack: &WindowCloseAck) -> Result<(), AuthorityError> {
        self.snapshot()?;
        self.state = self.state.apply_close_ack(ack)?;
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
    transient_unknown: Arc<AtomicU64>,
    consume_reply_budget: Arc<AtomicBool>,
    unknown_confirmation: Arc<AtomicBool>,
    unknown_close: Arc<AtomicBool>,
    close_requests: Arc<Mutex<Vec<WindowCloseRequest>>>,
    confirmation_budgets: Arc<Mutex<Vec<Duration>>>,
    recorded: Arc<AtomicBool>,
    capacity_refusal: Arc<AtomicBool>,
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
            transient_unknown: Arc::new(AtomicU64::new(0)),
            consume_reply_budget: Arc::new(AtomicBool::new(false)),
            unknown_confirmation: Arc::new(AtomicBool::new(false)),
            unknown_close: Arc::new(AtomicBool::new(false)),
            close_requests: Arc::new(Mutex::new(Vec::new())),
            confirmation_budgets: Arc::new(Mutex::new(Vec::new())),
            recorded: Arc::new(AtomicBool::new(false)),
            capacity_refusal: Arc::new(AtomicBool::new(false)),
            last: Arc::new(Mutex::new(None)),
            responses: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
            fence_after_send: Arc::new(Mutex::new(None)),
            gate: Arc::new(Mutex::new(None)),
        }
    }
}
impl HostPort for Host {
    fn close(
        &self,
        request: &WindowCloseRequest,
        _: Duration,
    ) -> Result<WindowCloseAck, PortError> {
        self.close_requests.lock().unwrap().push(request.clone());
        if self.unknown_close.load(Ordering::SeqCst) {
            return Err(PortError::OutcomeUnknown);
        }
        // This controlled unit port does NOT attest host persistence. A separate
        // real mTLS + original SQLite test exercises actual host-close storage.
        Ok(WindowCloseAck {
            terminal: request.terminal.clone(),
        })
    }
    fn confirm(
        &self,
        result: &LifecycleResponse,
        budget: Duration,
    ) -> Result<LifecycleResponse, PortError> {
        self.confirmation_budgets.lock().unwrap().push(budget);
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
        budget: Duration,
    ) -> Result<LifecycleResponse, PortError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last.lock().unwrap() = Some(request.clone());
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.entered.send(()).unwrap();
            gate.release.recv().unwrap();
        }
        if self.consume_reply_budget.load(Ordering::SeqCst) {
            consume_reply_budget(budget);
        }
        if self.unknown.load(Ordering::SeqCst)
            || self
                .transient_unknown
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
        {
            return Err(PortError::Busy);
        }
        let fact =
            if self.capacity_refusal.load(Ordering::SeqCst) && request.intent == Intent::Create {
                LifecycleFact::NotAccepted
            } else if self.recorded.load(Ordering::SeqCst) {
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
#[test]
fn confirmation_uses_remaining_attempt_budget_without_resetting_original_wait() {
    let f = fixture();
    let session = f.app.create("1", command()).unwrap();
    f.host.unknown_confirmation.store(true, Ordering::SeqCst);
    f.app.kill("1", &session.sandbox_id).unwrap();
    let state = f.app.snapshot().unwrap();
    let result = state.sandboxes()[0]
        .operations
        .last()
        .unwrap()
        .response
        .as_ref()
        .unwrap();
    assert!(
        f.app.snapshot().unwrap().sandboxes()[0]
            .terminal_confirmation
            .is_none()
    );
    f.host.unknown_confirmation.store(false, Ordering::SeqCst);
    f.app.reconcile_once().unwrap();
    assert_eq!(
        f.app.snapshot().unwrap().sandboxes()[0]
            .terminal_confirmation
            .as_ref(),
        Some(result)
    );
    assert!(
        f.host
            .confirmation_budgets
            .lock()
            .unwrap()
            .iter()
            .all(|budget| *budget < f.app.config.reply_timeout)
    );
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

#[test]
fn terminal_confirmation_without_compaction_retries_before_close_dispatch() {
    let f = fixture();
    let session = f.app.create("1", command()).unwrap();
    let node = f
        .host
        .last
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .identity
        .node
        .clone();
    f.host.unknown_confirmation.store(true, Ordering::SeqCst);
    f.app.kill("1", &session.sandbox_id).unwrap();
    let terminal = f.app.snapshot().unwrap();
    let record = &terminal.sandboxes()[0];
    assert_eq!(record.operations.len(), 2);
    assert!(!record.has_compaction_work());
    assert!(record.terminal_confirmation.is_none());
    let cleaned = record.operations.last().unwrap().response.clone().unwrap();
    assert_eq!(f.app.begin_close(&cleaned), Err(PortError::Conflict));
    assert!(f.host.close_requests.lock().unwrap().is_empty());
    f.host.unknown_confirmation.store(false, Ordering::SeqCst);
    f.app.reconcile_once().unwrap();
    let confirmed = f.app.snapshot().unwrap();
    assert_eq!(
        confirmed.sandboxes()[0].terminal_confirmation.as_ref(),
        confirmed.sandboxes()[0]
            .operations
            .last()
            .unwrap()
            .response
            .as_ref()
    );
    f.host.unknown_close.store(true, Ordering::SeqCst);
    let close = f.app.begin_close(&cleaned).unwrap();
    assert!(f.host.close_requests.lock().unwrap().is_empty()); // decision committed before detached I/O
    f.app.reconcile_once().unwrap();
    assert_eq!(
        f.app.snapshot().unwrap().pending_closes(),
        vec![close.clone()]
    );
    f.host.unknown_close.store(false, Ordering::SeqCst);
    f.app.reconcile_once().unwrap();
    let reopened = f.app.snapshot().unwrap();
    assert!(reopened.sandboxes().is_empty());
    assert_eq!(
        reopened
            .lane(&node, cleaned.identity.retention_slot)
            .unwrap()
            .current()
            .unwrap()
            .get(),
        2
    );
    assert_eq!(
        *f.host.close_requests.lock().unwrap(),
        vec![close.clone(), close]
    );
}

#[test]
fn one_operation_not_accepted_requires_exact_terminal_confirmation() {
    let f = fixture();
    f.host.unknown.store(true, Ordering::SeqCst);
    assert!(f.app.create("1", command()).is_err());
    let request = f.host.last.lock().unwrap().clone().unwrap();
    let terminal = LifecycleResponse {
        identity: request.identity.clone(),
        observation_version: 1.try_into().unwrap(),
        fact: LifecycleFact::NotAccepted,
    };
    f.app
        .authority
        .lock()
        .unwrap()
        .apply_response(&terminal)
        .unwrap();
    f.host.responses.lock().unwrap().insert(
        request.identity.operation_id.as_str().to_owned(),
        terminal.clone(),
    );
    f.host.unknown_confirmation.store(true, Ordering::SeqCst);
    f.app.reconcile_once().unwrap();
    assert!(
        f.app.snapshot().unwrap().sandboxes()[0]
            .terminal_confirmation
            .is_none()
    );
    assert_eq!(f.app.begin_close(&terminal), Err(PortError::Conflict));
    f.host.unknown_confirmation.store(false, Ordering::SeqCst);
    f.app.reconcile_once().unwrap();
    let state = f.app.snapshot().unwrap();
    assert_eq!(state.sandboxes()[0].operations.len(), 1);
    assert_eq!(
        state.sandboxes()[0].terminal_confirmation,
        Some(terminal.clone())
    );
    assert!(f.app.begin_close(&terminal).is_ok());
}

#[test]
fn terminal_lane_retirement_retries_exact_close_without_rotating_empty_successors() {
    let f = fixture();
    let session = f.app.create("1", command()).unwrap();
    let original = f.host.last.lock().unwrap().clone().unwrap();
    f.app.retire_terminal_history_once().unwrap();
    assert!(f.host.close_requests.lock().unwrap().is_empty());
    f.host.unknown_confirmation.store(true, Ordering::SeqCst);
    f.app.kill("1", &session.sandbox_id).unwrap();
    f.app.retire_terminal_history_once().unwrap();
    assert!(f.host.close_requests.lock().unwrap().is_empty());
    f.host.unknown_confirmation.store(false, Ordering::SeqCst);
    f.app.reconcile_once().unwrap();
    f.host.unknown_close.store(true, Ordering::SeqCst);
    f.app.retire_terminal_history_once().unwrap();
    let pending = f.app.snapshot().unwrap().pending_closes();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].terminal.identity.node, original.identity.node);
    assert_eq!(
        pending[0].terminal.identity.request_window,
        original.identity.request_window
    );
    f.host.unknown_close.store(false, Ordering::SeqCst);
    f.app.reconcile_once().unwrap();
    let successor = f.app.snapshot().unwrap();
    assert!(successor.sandboxes().is_empty());
    assert_eq!(
        successor
            .lane(&original.identity.node, original.identity.retention_slot)
            .unwrap()
            .current()
            .unwrap()
            .get(),
        2
    );
    let requests = f.host.close_requests.lock().unwrap().clone();
    assert_eq!(requests, vec![pending[0].clone(), pending[0].clone()]);
    f.app.retire_terminal_history_once().unwrap();
    assert_eq!(*f.host.close_requests.lock().unwrap(), requests);
    let next = f.app.create("1", command()).unwrap();
    assert_ne!(next.sandbox_id, session.sandbox_id);
    assert_eq!(
        f.host
            .last
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .identity
            .request_window
            .get(),
        2
    );
}

#[test]
fn background_retirement_keeps_a_live_sibling_and_its_original_data_token() {
    let f = fixture();
    let survivor = f.app.create("1", command()).unwrap();
    let original = f.app.snapshot().unwrap().sandboxes()[0].clone();
    let original_view = f.app.get("1", &survivor.sandbox_id).unwrap();
    for round in 1..=8 {
        let peer = f.app.create("1", command()).unwrap();
        let request = f.host.last.lock().unwrap().clone().unwrap();
        assert_eq!(request.identity.request_window.get(), round);
        assert_ne!(
            request.identity.retention_slot,
            original.operations[0].request.identity.retention_slot
        );
        f.app.kill("1", &peer.sandbox_id).unwrap();
        f.host.unknown_close.store(true, Ordering::SeqCst);
        f.app.retire_terminal_history_once().unwrap();
        let pending = f.app.snapshot().unwrap().pending_closes();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].terminal.identity.sandbox_id.as_str(),
            peer.sandbox_id
        );
        assert!(
            f.app
                .resolve_data_route(&survivor.envd_access_token)
                .is_ok()
        );
        f.host.unknown_close.store(false, Ordering::SeqCst);
        f.app.reconcile_once().unwrap();
        let snapshot = f.app.snapshot().unwrap();
        assert_eq!(snapshot.sandboxes(), std::slice::from_ref(&original));
        assert_eq!(f.app.get("1", &survivor.sandbox_id).unwrap(), original_view);
        assert!(
            f.app
                .resolve_data_route(&survivor.envd_access_token)
                .is_ok()
        );
    }
}

#[test]
fn durable_original_capacity_refusal_is_429_and_never_waits_for_a_fake_destroy() {
    let f = fixture();
    f.host.capacity_refusal.store(true, Ordering::SeqCst);
    assert_eq!(f.app.create("1", command()), Err(PortError::QuotaExceeded));
    let snapshot = f.app.snapshot().unwrap();
    let record = &snapshot.sandboxes()[0];
    assert_eq!(record.operations.len(), 1);
    let terminal = record.operations[0].response.as_ref().unwrap();
    assert_eq!(terminal.fact, LifecycleFact::NotAccepted);
    assert_eq!(record.terminal_confirmation.as_ref(), Some(terminal));
    assert_eq!(snapshot.reserved(None, None), Some(Resources::ZERO));
    f.app.retire_terminal_history_once().unwrap();
    assert!(f.app.snapshot().unwrap().sandboxes().is_empty());
    assert_eq!(f.host.calls.load(Ordering::SeqCst), 1);
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
                hidden_sandboxes: Vec::new(),
                unreadable: Arc::clone(&unreadable),
                unknown_create: Arc::clone(&unknown_create),
                projection_steps: std::collections::VecDeque::new(),
                projection_reads: 0,
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
fn private_validation_cannot_issue_public_tokens_or_receive_user_management() {
    let f = fixture();
    let hidden = f.app.create("1", command()).unwrap();
    let visible = f.app.create("1", command()).unwrap();
    f.app.authority.lock().unwrap().hidden_sandboxes =
        vec![hidden.sandbox_id.as_str().try_into().unwrap()];
    let calls = f.host.calls.load(Ordering::SeqCst);
    assert_eq!(f.app.get("1", &hidden.sandbox_id), Err(PortError::NotFound));
    assert_eq!(
        f.app.session_attempt("1", &hidden.sandbox_id),
        Err(PortError::NotFound)
    );
    assert_eq!(
        f.app.resolve_data_route(&hidden.envd_access_token),
        Err(PortError::NotFound)
    );
    assert_eq!(
        f.app
            .connect("1", &hidden.sandbox_id, ConnectCommand { timeout: None }),
        Err(PortError::NotFound)
    );
    assert_eq!(
        f.app.set_timeout("1", &hidden.sandbox_id, 10),
        Err(PortError::NotFound)
    );
    assert_eq!(
        f.app.kill("1", &hidden.sandbox_id),
        Err(PortError::NotFound)
    );
    assert_eq!(f.host.calls.load(Ordering::SeqCst), calls);
    let page = f
        .app
        .list(
            "1",
            sandproof_controller_service::application::ListSelection::default(),
        )
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].sandbox_id, visible.sandbox_id);
    assert_eq!(page.total_running, Some(1));
    assert!(f.app.resolve_data_route(&visible.envd_access_token).is_ok());
    assert_eq!(f.app.snapshot().unwrap().sandboxes().len(), 2);
    assert_eq!(
        f.app.snapshot().unwrap().reserved(None, None),
        Some(resources(2))
    );
    // Hiding is NOT cleanup. The trusted internal driver still sees the SAME
    // record and can commit its original Destroy; no public bypass is added.
    let destroy = f
        .app
        .authority
        .lock()
        .unwrap()
        .destroy(
            &"1".try_into().unwrap(),
            &hidden.sandbox_id.as_str().try_into().unwrap(),
        )
        .unwrap();
    assert!(f.app.wait(&destroy).is_ok());
    assert_eq!(
        f.app.snapshot().unwrap().reserved(None, None),
        Some(resources(1))
    );
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
fn stale_node_health_refuses_connect_and_positive_timeout_before_new_intent_or_dispatch() {
    for healthy_until in [0, 999, 1000] {
        let f = fixture();
        let session = f.app.create("1", command()).unwrap();
        {
            let mut authority = f.app.authority.lock().unwrap();
            let node = authority.state.nodes()[0].identity.clone();
            authority.state = authority
                .state
                .refresh_node(&node, healthy_until, Resources::ZERO)
                .unwrap();
        }
        let original = f.app.snapshot().unwrap();
        assert_eq!(
            f.app
                .connect("1", &session.sandbox_id, ConnectCommand::default()),
            Err(PortError::Busy)
        );
        assert_eq!(
            f.app.set_timeout("1", &session.sandbox_id, 100),
            Err(PortError::Busy)
        );
        assert_eq!(f.app.snapshot().unwrap(), original);
        assert_eq!(f.host.calls.load(Ordering::SeqCst), 1);
        // A failed time/health observation must not prevent exact cleanup.
        f.app.kill("1", &session.sandbox_id).unwrap();
        assert_eq!(
            f.app.snapshot().unwrap().reserved(None, None),
            Some(Resources::ZERO)
        );
    }
}

#[test]
fn unhealthy_node_still_accepts_nonpositive_expiry_and_reconciles_original_destroy() {
    for seconds in [0, -1, i64::MIN] {
        let f = fixture();
        let session = f.app.create("1", command()).unwrap();
        {
            let mut authority = f.app.authority.lock().unwrap();
            let node = authority.state.nodes()[0].identity.clone();
            authority.state = authority
                .state
                .refresh_node(&node, 0, Resources::ZERO)
                .unwrap();
        }
        f.app
            .set_timeout("1", &session.sandbox_id, seconds)
            .unwrap();
        assert!(f.app.snapshot().unwrap().sandboxes()[0].terminating);
        assert_eq!(f.host.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            f.app.snapshot().unwrap().reserved(None, None),
            Some(resources(1))
        );
        f.app.reconcile_once().unwrap();
        assert_eq!(
            f.app.snapshot().unwrap().reserved(None, None),
            Some(Resources::ZERO)
        );
        assert_eq!(f.host.calls.load(Ordering::SeqCst), 2);
    }
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
fn transient_host_budget_refusal_retries_only_the_original_durable_intent() {
    let f = fixture();
    f.host.transient_unknown.store(2, Ordering::SeqCst);
    let session = f.app.create("1", command()).unwrap();
    assert_eq!(f.host.calls.load(Ordering::SeqCst), 3);
    let state = f.app.snapshot().unwrap();
    assert_eq!(state.sandboxes().len(), 1);
    assert_eq!(state.sandboxes()[0].operations.len(), 1);
    let request = &state.sandboxes()[0].operations[0].request;
    assert_eq!(request.identity.sandbox_id.as_str(), session.sandbox_id);
    assert_eq!(request.identity.operation_id.as_str(), "operation-1");
    assert_eq!(f.host.last.lock().unwrap().as_ref(), Some(request));
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
