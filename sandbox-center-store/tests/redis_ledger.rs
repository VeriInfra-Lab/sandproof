#![cfg(unix)]

use sandproof_center_store::{
    ControlState, RedisEndpoint, RedisLedger, StoreConfig, StoreError, WriterLease,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    net::SocketAddrV4,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[path = "support/engine.rs"]
mod engine;
use engine::{Engine, KEY, MEMORY, connect, deployment, endpoint};

#[allow(
    dead_code,
    reason = "shared original application fixture serves selected component scenarios"
)]
#[path = "../../controller-service/tests/support/internal_application.rs"]
mod host_application;
use host_application::complete_fixture as local_fixture;
use host_application::wire as lifecycle_fixture;
use sandproof_center_core::{Bounds, CenterState, NodeMode, NodeRegistration, TenantBudget};
use sandproof_center_store::authority::{AuthorityError, CenterAuthority, LifecycleTransport};
use sandproof_protocol::internal::lifecycle::{
    Intent, LifecycleFact, LifecycleRequest, LifecycleResponse,
};

#[path = "redis_ledger/aof.rs"]
mod aof;
#[path = "redis_ledger/fixture_identity.rs"]
mod fixture_identity;
#[path = "redis_ledger/lifecycle.rs"]
mod lifecycle;
#[path = "redis_ledger/window.rs"]
mod window;

fn center_state() -> CenterState {
    center_state_with_resources(
        lifecycle_fixture::resources(2),
        &lifecycle_fixture::binding(),
    )
}
fn center_state_with_resources(
    resources: sandproof_protocol::internal::lifecycle::Resources,
    binding: &sandproof_protocol::internal::lifecycle::CreateBinding,
) -> CenterState {
    let node = lifecycle_fixture::node("host-a");
    CenterState::new(
        deployment(),
        Bounds {
            max_nodes: 1,
            max_templates_per_node: 16,
            max_sandboxes: 2,
            operations_per_sandbox: 3,
            max_options_bytes: 128,
            platform: resources,
        },
        vec![NodeRegistration {
            retention_slots: 2,
            role: sandproof_center_core::NodeRole::Runtime,
            identity: node,
            limit: resources,
            mode: NodeMode::Accepting,
            healthy_until_ms: 2000,
            free: resources,
            templates: vec![sandproof_center_core::AvailableTemplate {
                revision: binding.template_revision.clone(),
                digest: binding.template_digest,
            }],
            protocol_version: 2,
        }],
        vec![TenantBudget {
            tenant: "1".try_into().unwrap(),
            limit: resources,
        }],
    )
    .unwrap()
}
fn center_authority(fixture: &Engine, address: SocketAddrV4) -> CenterAuthority {
    center_authority_with_state(fixture, address, &center_state())
}
fn center_authority_with_state(
    fixture: &Engine,
    address: SocketAddrV4,
    state: &CenterState,
) -> CenterAuthority {
    let limit = state.bounds().required_block_bytes();
    let (ledger, _) = RedisLedger::initialize(
        endpoint(address),
        Engine::config(limit),
        fixture.lease(),
        state,
    )
    .unwrap();
    CenterAuthority::attach(ledger).unwrap()
}

struct InboxTransport<'a>(&'a mut sandproof_controller_service::ControllerJournal);
impl LifecycleTransport for InboxTransport<'_> {
    type Error = sandproof_controller_service::JournalError;
    async fn send(&mut self, request: &LifecycleRequest) -> Result<LifecycleResponse, Self::Error> {
        self.0
            .record_internal_intent(request, &mut sandproof_controller_service::NeverCrash)
    }
}

#[path = "../../sandbox-internal/tests/support/tls.rs"]
mod tls_fixture;

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one ordered real Redis/HTTPS/SQLite lost-reply scenario"
)]
async fn committed_outbox_real_https_sqlite_and_lost_reply_preserve_one_operation() {
    use sandproof_internal::{
        ClientTls, InternalClient, InternalServer, LifecycleReceiver, Limits, ReceiveError,
        RegisteredEndpoint, ServerTls,
    };
    struct LoseFirst {
        inbox: sandproof_host_service::HostLifecycle<host_application::TestHost>,
        lose: AtomicBool,
    }
    impl sandproof_internal::ValidationReceiver for LoseFirst {
        async fn validation_evidence(
            &self,
            request: sandproof_internal::template::ValidationRequest,
        ) -> Result<sandproof_internal::template::ValidationEvidence, ReceiveError> {
            self.inbox.validation_evidence(request).await
        }
    }
    impl LifecycleReceiver for LoseFirst {
        async fn close(
            &self,
            request: sandproof_protocol::internal::window::WindowCloseRequest,
        ) -> Result<sandproof_protocol::internal::window::WindowCloseAck, ReceiveError> {
            self.inbox.close(request).await
        }
        async fn confirm(
            &self,
            result: LifecycleResponse,
        ) -> Result<LifecycleResponse, ReceiveError> {
            self.inbox.confirm(result).await
        }
        async fn receive(
            &self,
            request: LifecycleRequest,
        ) -> Result<LifecycleResponse, ReceiveError> {
            let response = self.inbox.receive(request).await?;
            // Actual SQLite commit happened, but no receipt reaches the center.
            if self.lose.swap(false, Ordering::SeqCst) {
                Err(ReceiveError::Unavailable)
            } else {
                Ok(response)
            }
        }
    }
    impl sandproof_internal::DataReceiver for LoseFirst {
        async fn receive_data(
            &self,
            _: sandproof_protocol::internal::ControlIdentity,
            _: axum::http::Request<axum::body::Body>,
        ) -> axum::http::Response<axum::body::Body> {
            // This storage/lifecycle fixture has no guest data handler, never a
            // production default implementation or synthetic data success.
            let mut response = axum::http::Response::new(axum::body::Body::empty());
            *response.status_mut() = axum::http::StatusCode::SERVICE_UNAVAILABLE;
            response
        }
    }
    let fixture = Engine::new();
    let binding = host_application::request().create;
    let resources = binding.resources.checked_add(binding.resources).unwrap();
    let state = center_state_with_resources(resources, &binding);
    let limit = state.bounds().required_block_bytes();
    let mut center = center_authority_with_state(&fixture, fixture.address, &state);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let physical = Arc::new(std::sync::Mutex::new(host_application::State::default()));
    let clock = Arc::new(host_application::TestClock(
        std::sync::atomic::AtomicI64::new(10),
    ));
    let host = host_application::application(&path, Arc::clone(&physical), clock, true);
    let certs = tls_fixture::certificates();
    let limits = Limits::new(4, Duration::from_secs(2), Duration::from_secs(5)).unwrap();
    let inbox = sandproof_host_service::HostLifecycle::new(Arc::clone(&host));
    let server = InternalServer::new(
        ServerTls::new(
            certs.host.material(),
            vec![certs.ca.der().clone()],
            vec![certs.center.pin()],
        )
        .unwrap(),
        lifecycle_fixture::node("host-a"),
        limits,
        sandproof_internal::diagnostic::RejectTemplateCache::new(LoseFirst {
            inbox: inbox.clone(),
            lose: AtomicBool::new(true),
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = Arc::new(InternalClient::new(
        ClientTls::new(
            certs.center.material(),
            vec![certs.ca.der().clone()],
            vec![certs.host.pin()],
        )
        .unwrap(),
        RegisteredEndpoint::new(
            listener.local_addr().unwrap(),
            "host.test".into(),
            lifecycle_fixture::node("host-a"),
        )
        .unwrap(),
        limits,
    ));
    let (stop, shutdown) = tokio::sync::oneshot::channel();
    let run = server.clone();
    let task = tokio::spawn(async move { run.serve(listener, shutdown).await });
    let mut transport = |request: LifecycleRequest| {
        let client = Arc::clone(&client);
        async move { client.lifecycle(&request).await }
    };
    let create = center.create(&"1".try_into().unwrap(), binding, 0).unwrap();
    assert!(
        center
            .dispatch_batch()
            .unwrap()
            .pop()
            .unwrap()
            .send(&mut transport)
            .await
            .is_err()
    );
    assert_eq!(center.state().unwrap().pending(), vec![create.clone()]);
    assert_eq!(
        center.state().unwrap().reserved(None, None),
        Some(create.create.resources)
    );
    assert_eq!(
        host.internal_pending(&lifecycle_fixture::node("host-a"))
            .unwrap(),
        vec![create.clone()]
    );
    let response = center
        .dispatch_batch()
        .unwrap()
        .pop()
        .unwrap()
        .send(&mut transport)
        .await
        .unwrap();
    assert_eq!(response.fact, LifecycleFact::Recorded);
    center.apply_response(&response).unwrap();
    let replay = center
        .dispatch_batch()
        .unwrap()
        .pop()
        .unwrap()
        .send(&mut transport)
        .await
        .unwrap();
    assert_eq!(response, replay);
    assert!(physical.lock().unwrap().actions.is_empty());
    // Original VM workflow/fact is durable, but the center never receives Ready.
    let ready = inbox
        .advance(
            create.clone(),
            Some(host_application::template()),
            host_application::compute(),
        )
        .await
        .unwrap();
    assert!(matches!(ready.fact, LifecycleFact::Ready { .. }));
    drop(center);
    let (ledger, _) = RedisLedger::<CenterState>::open(
        endpoint(fixture.address),
        Engine::config(limit),
        fixture.lease(),
    )
    .unwrap();
    let mut center = CenterAuthority::attach(ledger).unwrap();
    let replay_ready = center
        .dispatch_batch()
        .unwrap()
        .pop()
        .unwrap()
        .send(&mut transport)
        .await
        .unwrap();
    assert_eq!(replay_ready, ready);
    center.apply_response(&replay_ready).unwrap();
    assert_eq!(
        physical
            .lock()
            .unwrap()
            .actions
            .iter()
            .filter(|a| matches!(
                a.command.command(),
                sandbox_controller_core::Command::PrepareVm { .. }
            ))
            .count(),
        1
    );
    let destroy = center
        .destroy(&"1".try_into().unwrap(), &create.identity.sandbox_id)
        .unwrap();
    let response = center
        .dispatch_batch()
        .unwrap()
        .pop()
        .unwrap()
        .send(&mut transport)
        .await
        .unwrap();
    center.apply_response(&response).unwrap();
    assert_eq!(
        host.internal_pending(&lifecycle_fixture::node("host-a"))
            .unwrap(),
        vec![destroy.clone()]
    );
    assert!(center.apply_response(&replay).is_err());
    assert_eq!(
        center.state().unwrap().reserved(None, None),
        Some(create.create.resources)
    );
    let cleaned = inbox
        .advance(destroy, None, host_application::compute())
        .await
        .unwrap();
    assert_eq!(cleaned.fact, LifecycleFact::Cleaned);
    let response = center
        .dispatch_batch()
        .unwrap()
        .pop()
        .unwrap()
        .send(&mut transport)
        .await
        .unwrap();
    assert_eq!(response, cleaned);
    center.apply_response(&response).unwrap();
    assert_eq!(
        center.state().unwrap().reserved(None, None),
        Some(sandproof_protocol::internal::lifecycle::Resources::ZERO)
    );
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(inbox.active_work(), 0);
    assert_eq!(server.active_connections(), 0);
    assert_eq!(client.active_calls(), 0);
}

#[tokio::test]
async fn real_redis_commit_dispatch_sqlite_record_and_replay_are_one_identity_not_vm_success() {
    let fixture = Engine::new();
    let mut center = center_authority(&fixture, fixture.address);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let namespace = "0123456789abcdef";
    let mut host = sandproof_controller_service::ControllerJournal::open_complete(
        &path,
        namespace,
        local_fixture::host(&path, namespace, 64, 100_000, local_fixture::CLEANUP_BYTES),
        local_fixture::maintenance(),
        0,
    )
    .unwrap();
    host.initialize_internal_inbox(
        sandproof_controller_service::InternalInboxConfig {
            node: lifecycle_fixture::node("host-a"),
            max_sandboxes: 2,
            operations_per_sandbox: 3,
            max_options_bytes: 128,
        },
        &mut sandproof_controller_service::NeverCrash,
    )
    .unwrap();
    let create = center
        .create(&"1".try_into().unwrap(), lifecycle_fixture::binding(), 0)
        .unwrap();
    let batch = center.dispatch_batch().unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].request(), &create);
    let answer = batch
        .into_iter()
        .next()
        .unwrap()
        .send(&mut InboxTransport(&mut host))
        .await
        .unwrap();
    assert_eq!(answer.fact, LifecycleFact::Recorded);
    center.apply_response(&answer).unwrap();
    let replay = center
        .dispatch_batch()
        .unwrap()
        .pop()
        .unwrap()
        .send(&mut InboxTransport(&mut host))
        .await
        .unwrap();
    assert_eq!(answer, replay);
    assert!(host.world().unwrap().sandbox_records().is_empty());
    assert!(
        center.state().unwrap().sandboxes()[0]
            .started_at_ms
            .is_none()
    );
    let destroy = center
        .destroy(&"1".try_into().unwrap(), &create.identity.sandbox_id)
        .unwrap();
    let answer = center
        .dispatch_batch()
        .unwrap()
        .pop()
        .unwrap()
        .send(&mut InboxTransport(&mut host))
        .await
        .unwrap();
    center.apply_response(&answer).unwrap();
    assert_eq!(
        host.internal_pending(&lifecycle_fixture::node("host-a"))
            .unwrap(),
        vec![destroy]
    );
    assert_eq!(
        center.state().unwrap().reserved(None, None),
        Some(lifecycle_fixture::resources(1))
    );
    let old = host
        .record_internal_intent(&create, &mut sandproof_controller_service::NeverCrash)
        .unwrap();
    assert_eq!(old, replay);
    assert!(matches!(
        center.apply_response(&old),
        Err(AuthorityError::Decision(_))
    ));
    assert_eq!(
        host.internal_pending(&lifecycle_fixture::node("host-a"))
            .unwrap()[0]
            .intent,
        Intent::Destroy
    );
}

#[test]
fn center_actual_commit_reply_loss_blocks_dispatch_then_recovers_same_outbox() {
    let fixture = Engine::new();
    let proxy = ReplyLoss::new(fixture.address);
    let mut center = center_authority(&fixture, proxy.address);
    proxy.lose_after(0);
    assert_eq!(
        center.create(&"1".try_into().unwrap(), lifecycle_fixture::binding(), 0),
        Err(AuthorityError::Store(StoreError::OutcomeUnknown))
    );
    assert!(matches!(
        center.dispatch_batch(),
        Err(AuthorityError::RecoveryRequired)
    ));
    assert!(center.state().is_err());
    center.recover().unwrap();
    let pending = center.state().unwrap().pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].identity.sandbox_id.as_str(), "sandbox-1");
    assert_eq!(pending[0].identity.operation_id.as_str(), "operation-1");
    assert_eq!(center.dispatch_batch().unwrap()[0].request(), &pending[0]);
}

#[tokio::test]
async fn host_transport_failure_and_old_fact_do_not_release_or_reroute() {
    struct Failed;
    impl LifecycleTransport for Failed {
        type Error = ();
        async fn send(&mut self, _: &LifecycleRequest) -> Result<LifecycleResponse, ()> {
            Err(())
        }
    }
    let fixture = Engine::new();
    let mut center = center_authority(&fixture, fixture.address);
    let create = center
        .create(&"1".try_into().unwrap(), lifecycle_fixture::binding(), 0)
        .unwrap();
    assert!(
        center
            .dispatch_batch()
            .unwrap()
            .pop()
            .unwrap()
            .send(&mut Failed)
            .await
            .is_err()
    );
    assert_eq!(center.state().unwrap().pending(), vec![create.clone()]);
    let destroy = center
        .destroy(&"1".try_into().unwrap(), &create.identity.sandbox_id)
        .unwrap();
    assert_eq!(destroy.intent, Intent::Destroy);
    let old = lifecycle_fixture::response(
        &create,
        1,
        LifecycleFact::Ready {
            started_at_ms: 100,
            end_at_ms: 200,
            max_end_at_ms: 600,
        },
    );
    assert!(matches!(
        center.apply_response(&old),
        Err(AuthorityError::Decision(_))
    ));
    assert_eq!(center.state().unwrap().pending(), vec![destroy]);
    assert_eq!(
        center.state().unwrap().reserved(None, None),
        Some(lifecycle_fixture::resources(1))
    );
    fixture.field("payload", b"corrupted");
    assert!(center.dispatch_batch().is_err());
    assert!(center.dispatch_batch().is_err());
}

#[test]
fn full_future_envelope_is_required_before_any_admission() {
    let fixture = Engine::new();
    let (ledger, _) = RedisLedger::initialize(
        endpoint(fixture.address),
        Engine::config(16_384),
        fixture.lease(),
        &center_state(),
    )
    .unwrap();
    assert!(matches!(
        CenterAuthority::attach(ledger),
        Err(AuthorityError::Store(StoreError::Configuration))
    ));
}

#[test]
fn slow_detached_transport_does_not_hold_center_decision_lock() {
    use std::sync::{Mutex, mpsc};
    struct Slow {
        entered: mpsc::SyncSender<()>,
        release: mpsc::Receiver<()>,
    }
    impl LifecycleTransport for Slow {
        type Error = ();
        async fn send(&mut self, request: &LifecycleRequest) -> Result<LifecycleResponse, ()> {
            self.entered.send(()).map_err(|_| ())?;
            self.release
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| ())?;
            Ok(lifecycle_fixture::response(
                request,
                1,
                LifecycleFact::Ready {
                    started_at_ms: 100,
                    end_at_ms: 200,
                    max_end_at_ms: 600,
                },
            ))
        }
    }
    let fixture = Engine::new();
    let center = Arc::new(Mutex::new(center_authority(&fixture, fixture.address)));
    let create = center
        .lock()
        .unwrap()
        .create(&"1".try_into().unwrap(), lifecycle_fixture::binding(), 0)
        .unwrap();
    let dispatch = center
        .lock()
        .unwrap()
        .dispatch_batch()
        .unwrap()
        .pop()
        .unwrap();
    let (entered, wait) = mpsc::sync_channel(0);
    let (release, gate) = mpsc::sync_channel(0);
    let worker = thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(dispatch.send(&mut Slow {
                entered,
                release: gate,
            }))
    });
    wait.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut authority = center
        .try_lock()
        .expect("no authority borrow/lock spans detached RPC");
    authority
        .destroy(&"1".try_into().unwrap(), &create.identity.sandbox_id)
        .unwrap();
    drop(authority);
    release.send(()).unwrap();
    let late = worker.join().unwrap().unwrap();
    assert!(matches!(
        center.lock().unwrap().apply_response(&late),
        Err(AuthorityError::Decision(_))
    ));
    assert_eq!(
        center.lock().unwrap().state().unwrap().reserved(None, None),
        Some(lifecycle_fixture::resources(1))
    );
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    frontier: u64,
    pending: String,
}
impl ControlState for State {
    const FORMAT_VERSION: u32 = 1;
    fn is_valid(&self) -> bool {
        self.frontier > 0
    }
}
fn state(value: u64) -> State {
    State {
        frontier: value,
        pending: "original-operation".into(),
    }
}
impl Engine {
    fn initialize(
        &self,
        limit: usize,
    ) -> (RedisLedger<State>, sandproof_center_store::Observed<State>) {
        let (ledger, committed) = RedisLedger::initialize(
            endpoint(self.address),
            Self::config(limit),
            self.lease(),
            &state(1),
        )
        .unwrap();
        (ledger, committed.into_observed())
    }
}

#[test]
fn atomic_commit_conflict_and_reaffirm_keep_original_binding() {
    let fixture = Engine::new();
    let (mut ledger, first) = fixture.initialize(1024);
    let second = ledger.commit(&first, &state(2)).unwrap();
    assert_eq!(second.revision().get(), 2);
    assert!(matches!(
        ledger.commit(&first, &state(3)),
        Err(StoreError::Conflict)
    ));
    let observed = ledger.read().unwrap();
    assert_eq!(observed.state(), &state(2));
    let recovered = ledger.reaffirm(&observed).unwrap();
    assert_eq!(recovered.revision().get(), 3);
    assert_eq!(recovered.state(), observed.state());
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg("*")
        .query(&mut connect(fixture.address))
        .unwrap();
    assert_eq!(keys, [KEY]);
    let fields: usize = redis::cmd("HLEN")
        .arg(KEY)
        .query(&mut connect(fixture.address))
        .unwrap();
    assert_eq!(fields, 4);
}

#[test]
fn ordinary_open_never_initializes_missing_or_overwrites_foreign_data() {
    let fixture = Engine::new();
    assert!(matches!(
        RedisLedger::<State>::open(
            endpoint(fixture.address),
            Engine::config(1024),
            fixture.lease()
        ),
        Err(StoreError::Missing)
    ));
    let _: () = redis::cmd("SET")
        .arg(KEY)
        .arg("foreign-fixture")
        .query(&mut connect(fixture.address))
        .unwrap();
    assert!(matches!(
        RedisLedger::initialize(
            endpoint(fixture.address),
            Engine::config(1024),
            fixture.lease(),
            &state(1)
        ),
        Err(StoreError::AlreadyInitialized)
    ));
    assert!(matches!(
        RedisLedger::<State>::open(
            endpoint(fixture.address),
            Engine::config(1024),
            fixture.lease()
        ),
        Err(StoreError::InvalidRecord)
    ));
    let unchanged: String = redis::cmd("GET")
        .arg(KEY)
        .query(&mut connect(fixture.address))
        .unwrap();
    assert_eq!(unchanged, "foreign-fixture");
}

#[test]
fn payload_capacity_and_same_revision_substitution_refuse_without_mutation() {
    let fixture = Engine::new();
    let length = serde_json::to_vec(&state(1)).unwrap().len();
    let (mut ledger, original) = fixture.initialize(length);
    let mut large = state(2);
    large.pending.push('x');
    assert!(matches!(
        ledger.commit(&original, &large),
        Err(StoreError::Capacity)
    ));
    assert!(matches!(
        ledger.commit(&original, &state(0)),
        Err(StoreError::InvalidState)
    ));
    assert_eq!(fixture.revision(), "1");
    fixture.field("payload", serde_json::to_vec(&state(9)).unwrap());
    assert!(matches!(
        ledger.commit(&original, &state(2)),
        Err(StoreError::InvalidRecord)
    ));
    assert_eq!(fixture.revision(), "1");
}

#[test]
fn malformed_metadata_payload_and_ttl_fail_closed() {
    let fixture = Engine::new();
    let (mut ledger, _) = fixture.initialize(1024);
    for (field, original, bad) in [
        ("deployment", "fixture-deployment", "foreign"),
        ("format", "1", "2"),
        ("revision", "1", "0"),
        ("revision", "1", "01"),
        ("revision", "1", "18446744073709551616"),
    ] {
        fixture.field(field, bad);
        assert!(matches!(ledger.read(), Err(StoreError::InvalidRecord)));
        fixture.field(field, original);
    }
    for payload in [
        "{",
        "null",
        r#"{"frontier":1,"frontier":2,"pending":"x"}"#,
        r#"{"frontier":1,"pending":"x","extra":0}"#,
        r#"{"frontier":0,"pending":"x"}"#,
    ] {
        fixture.field("payload", payload);
        assert!(matches!(
            ledger.read(),
            Err(StoreError::InvalidRecord | StoreError::InvalidState)
        ));
    }
    fixture.field("payload", "x".repeat(1025));
    assert!(matches!(ledger.read(), Err(StoreError::InvalidRecord)));
    fixture.field("payload", serde_json::to_vec(&state(1)).unwrap());
    fixture.field("extra", "unexpected");
    assert!(matches!(ledger.read(), Err(StoreError::InvalidRecord)));
    let _: usize = redis::cmd("HDEL")
        .arg(KEY)
        .arg("extra")
        .query(&mut connect(fixture.address))
        .unwrap();
    let _: bool = redis::cmd("PEXPIRE")
        .arg(KEY)
        .arg(60_000)
        .query(&mut connect(fixture.address))
        .unwrap();
    assert!(matches!(ledger.read(), Err(StoreError::InvalidRecord)));
}

#[test]
fn revision_above_lua_integer_precision_is_exact_and_exhaustion_never_wraps() {
    let fixture = Engine::new();
    let (mut ledger, _) = fixture.initialize(1024);
    fixture.field("revision", 9_007_199_254_740_993_u64);
    let observed = ledger.read().unwrap();
    assert_eq!(
        ledger
            .commit(&observed, &state(2))
            .unwrap()
            .revision()
            .get(),
        9_007_199_254_740_994
    );
    fixture.field("revision", u64::MAX);
    let observed = ledger.read().unwrap();
    assert!(matches!(
        ledger.commit(&observed, &state(3)),
        Err(StoreError::RevisionExhausted)
    ));
    assert_eq!(fixture.revision(), u64::MAX.to_string());
}

#[test]
fn every_required_persistence_setting_is_checked_before_write() {
    let fixture = Engine::new();
    let (mut ledger, original) = fixture.initialize(1024);
    for (name, good, bad) in [
        ("appendfsync", "always", "everysec"),
        ("maxmemory-policy", "noeviction", "allkeys-lru"),
        ("no-appendfsync-on-rewrite", "no", "yes"),
        ("aof-load-truncated", "no", "yes"),
        ("maxmemory", "67108864", "67108863"),
    ] {
        let _: () = redis::cmd("CONFIG")
            .arg("SET")
            .arg(name)
            .arg(bad)
            .query(&mut connect(fixture.address))
            .unwrap();
        assert!(matches!(
            ledger.commit(&original, &state(2)),
            Err(StoreError::Configuration)
        ));
        assert_eq!(fixture.revision(), "1");
        let _: () = redis::cmd("CONFIG")
            .arg("SET")
            .arg(name)
            .arg(good)
            .query(&mut connect(fixture.address))
            .unwrap();
    }
    let _: () = redis::cmd("CONFIG")
        .arg("SET")
        .arg("appendonly")
        .arg("no")
        .query(&mut connect(fixture.address))
        .unwrap();
    assert!(matches!(
        ledger.commit(&original, &state(2)),
        Err(StoreError::Configuration)
    ));
    assert_eq!(fixture.revision(), "1");
}

#[test]
fn engine_crash_recovers_same_state_and_requires_explicit_reaffirm() {
    let mut fixture = Engine::new();
    let (mut ledger, original) = fixture.initialize(1024);
    ledger.commit(&original, &state(2)).unwrap();
    drop(ledger);
    fixture.stop();
    fixture.start();
    let (mut recovered, observed) = RedisLedger::<State>::open(
        endpoint(fixture.address),
        Engine::config(1024),
        fixture.lease(),
    )
    .unwrap();
    assert_eq!(observed.state(), &state(2));
    assert_eq!(observed.revision().get(), 2);
    assert_eq!(recovered.reaffirm(&observed).unwrap().revision().get(), 3);
}

#[test]
fn truncated_aof_refuses_start_without_repairing_the_original_file() {
    let mut fixture = Engine::new();
    let (ledger, _) = fixture.initialize(1024);
    drop(ledger);
    fixture.stop();
    let path = fs::read_dir(fixture.root.path().join("appendonlydir"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with(".incr.aof"))
        .unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(b"*3\r\n$3\r\nSET\r\n$7\r\nbroken").unwrap();
    file.sync_all().unwrap();
    let bytes = fs::read(&path).unwrap();
    fixture.spawn();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(exit) = fixture.child.as_mut().unwrap().try_wait().unwrap() {
            assert!(!exit.success());
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    fixture.child = None; // reaped above; never kill an unrelated process
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(matches!(
        RedisLedger::<State>::open(
            endpoint(fixture.address),
            Engine::config(1024),
            fixture.lease()
        ),
        Err(StoreError::Unavailable)
    ));
}

#[path = "support/reply_loss.rs"]
mod reply_loss;
use reply_loss::ReplyLoss;

#[test]
fn actual_commit_reply_loss_fences_until_original_state_is_durably_reaffirmed() {
    let fixture = Engine::new();
    let proxy = ReplyLoss::new(fixture.address);
    let (mut ledger, committed) = RedisLedger::initialize(
        endpoint(proxy.address),
        Engine::config(1024),
        fixture.lease(),
        &state(1),
    )
    .unwrap();
    let original = committed.into_observed();
    proxy.armed.store(1, Ordering::SeqCst);
    assert!(matches!(
        ledger.commit(&original, &state(2)),
        Err(StoreError::OutcomeUnknown)
    ));
    assert!(ledger.writes_fenced());
    assert_eq!(fixture.revision(), "2");
    assert!(matches!(
        ledger.commit(&original, &state(3)),
        Err(StoreError::WriteFenced)
    ));
    ledger.reconnect().unwrap();
    let observed = ledger.read().unwrap();
    assert_eq!(observed.state(), &state(2));
    assert!(ledger.writes_fenced());
    assert!(matches!(
        ledger.commit(&observed, &state(3)),
        Err(StoreError::WriteFenced)
    ));
    let confirmed = ledger.reaffirm(&observed).unwrap();
    assert_eq!(confirmed.revision().get(), 3);
    assert_eq!(confirmed.state(), &state(2));
    assert!(!ledger.writes_fenced());
    drop(ledger); // release connection before joining proxy
}

#[test]
fn actual_noeviction_write_refusal_keeps_original_record_and_requires_reconciliation() {
    let fixture = Engine::new();
    let proxy = ReplyLoss::new(fixture.address);
    let (mut ledger, committed) = RedisLedger::initialize(
        endpoint(proxy.address),
        Engine::config(1024),
        fixture.lease(),
        &state(1),
    )
    .unwrap();
    proxy.armed.store(2, Ordering::SeqCst);
    assert!(matches!(
        ledger.commit(&committed.into_observed(), &state(2)),
        Err(StoreError::OutcomeUnknown)
    ));
    assert!(ledger.writes_fenced());
    assert_eq!(fixture.revision(), "1");
    let _: () = redis::cmd("CONFIG")
        .arg("SET")
        .arg("maxmemory")
        .arg(MEMORY)
        .query(&mut connect(fixture.address))
        .unwrap();
    ledger.reconnect().unwrap();
    let observed = ledger.read().unwrap();
    assert_eq!(observed.state(), &state(1));
    assert_eq!(ledger.reaffirm(&observed).unwrap().revision().get(), 2);
    drop(ledger);
}

#[test]
fn bad_acl_or_writer_deployment_cannot_open_or_mutate_the_authority() {
    let fixture = Engine::new();
    let (ledger, _) = fixture.initialize(1024);
    drop(ledger);
    let bad = RedisEndpoint::new(
        fixture.address,
        "default".into(),
        "incorrect-fixture".into(),
    )
    .unwrap();
    assert!(matches!(
        RedisLedger::<State>::open(bad, Engine::config(1024), fixture.lease()),
        Err(StoreError::Unavailable)
    ));
    let foreign = WriterLease::acquire(
        &fixture.root.path().join("writer"),
        "foreign-deployment".try_into().unwrap(),
    )
    .unwrap();
    assert!(matches!(
        RedisLedger::<State>::open(endpoint(fixture.address), Engine::config(1024), foreign),
        Err(StoreError::Configuration)
    ));
    assert_eq!(fixture.revision(), "1");
}

#[test]
fn stalled_real_engine_has_finite_operation_deadline_and_no_automatic_reconnect() {
    let fixture = Engine::new();
    let (ledger, _) = fixture.initialize(1024);
    drop(ledger);
    let config = StoreConfig::new(deployment(), 1024, MEMORY, Duration::from_millis(50)).unwrap();
    let (mut ledger, _) =
        RedisLedger::<State>::open(endpoint(fixture.address), config, fixture.lease()).unwrap();
    let mut admin = connect(fixture.address);
    let _: () = redis::cmd("CLIENT")
        .arg("PAUSE")
        .arg(300)
        .arg("ALL")
        .query(&mut admin)
        .unwrap();
    let start = Instant::now();
    assert!(matches!(ledger.read(), Err(StoreError::Unavailable)));
    assert!(start.elapsed() < Duration::from_secs(1));
    let _: () = redis::cmd("CLIENT")
        .arg("UNPAUSE")
        .query(&mut admin)
        .unwrap();
    assert!(matches!(ledger.read(), Err(StoreError::Unavailable))); // old stream discarded
    ledger.reconnect().unwrap();
    assert_eq!(ledger.read().unwrap().state(), &state(1));
}
