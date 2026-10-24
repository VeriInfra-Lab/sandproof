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

use current_application::fixture as host_application;
use host_application::wire as lifecycle_fixture;
use sandproof_center_core::{Bounds, CenterState, NodeMode, NodeRegistration, TenantBudget};
use sandproof_center_core::{CreateParameters, LifecycleSubmission, LifecycleWindow};
use sandproof_center_store::authority::{AuthorityError, CenterAuthority};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, ClosedLifecycleWindow,
    ConfirmedLifecycleResult, CurrentCreateReadiness, LifecycleOperation, OperationKey,
    TerminalLifecycleObservation, TerminalStatus,
};
#[allow(
    dead_code,
    reason = "shared current application fixture serves selected component scenarios"
)]
#[path = "../../controller-service/tests/support/lifecycle_application.rs"]
mod current_application;

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
            result_retention_ms: 100,
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
        ["1", "2"]
            .into_iter()
            .map(|tenant| TenantBudget {
                tenant: tenant.try_into().unwrap(),
                limit: resources,
            })
            .collect(),
        current_application::config().center,
        ["1", "2"]
            .into_iter()
            .map(|owner| LifecycleWindow {
                owner: owner.try_into().unwrap(),
                window: 9.try_into().unwrap(),
            })
            .collect(),
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
    let limit = state.required_authority_bytes();
    let (ledger, _) = RedisLedger::initialize(
        endpoint(address),
        Engine::config(limit),
        fixture.lease(),
        state,
    )
    .unwrap();
    CenterAuthority::attach(ledger).unwrap()
}

#[path = "redis_ledger/components.rs"]
mod components;

fn create_submission(
    binding: sandproof_protocol::internal::lifecycle::CreateBinding,
    sequence: u64,
) -> LifecycleSubmission {
    LifecycleSubmission::Create {
        key: OperationKey {
            deployment: deployment(),
            owner: "1".try_into().unwrap(),
            window: 9.try_into().unwrap(),
            sequence: sequence.try_into().unwrap(),
        },
        parameters: CreateParameters {
            template_revision: binding.template_revision,
            template_digest: binding.template_digest,
            resources: binding.resources,
            options: binding.options,
            startup_timeout_ms: 1000,
            timeout_ms: binding.timeout_ms,
            max_lifetime_ms: binding.max_lifetime_ms,
        },
    }
}
fn accept_create(
    center: &mut CenterAuthority,
    binding: sandproof_protocol::internal::lifecycle::CreateBinding,
    sequence: u64,
) -> AcceptedLifecycleIntent {
    let owner = "1".try_into().unwrap();
    let mut submission = create_submission(binding, sequence);
    let LifecycleSubmission::Create { key, .. } = &mut submission else {
        unreachable!()
    };
    key.window = center
        .state()
        .unwrap()
        .public_request_window(&owner)
        .unwrap()
        .unwrap();
    center.accept_lifecycle(&owner, &submission, 10).unwrap()
}
fn accept_destroy(
    center: &mut CenterAuthority,
    create: &AcceptedLifecycleIntent,
    sequence: u64,
) -> AcceptedLifecycleIntent {
    let mut key = create.request.binding.key.clone();
    key.sequence = sequence.try_into().unwrap();
    center
        .accept_lifecycle(
            &key.owner.clone(),
            &LifecycleSubmission::Destroy {
                key,
                session: create.request.binding.session.clone(),
                expected_version: create.accepted_version,
            },
            20,
        )
        .unwrap()
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
fn actual_commit_reply_timeout_fences_until_original_state_is_durably_reaffirmed() {
    let fixture = Engine::new();
    let proxy = ReplyLoss::new(fixture.address);
    let (mut ledger, committed) = RedisLedger::initialize(
        endpoint(proxy.address),
        Engine::config(1024),
        fixture.lease(),
        &state(1),
    ).unwrap();
    let original = committed.into_observed();
    proxy.timeout_after(0);
    let result = ledger.commit(&original, &state(2));
    proxy.release_timeout();
    assert!(matches!(result, Err(StoreError::OutcomeUnknown)));
    assert!(ledger.writes_fenced());
    assert_eq!(fixture.revision(), "2");
    assert!(matches!(ledger.commit(&original, &state(3)), Err(StoreError::WriteFenced)));
    ledger.reconnect().unwrap();
    let observed = ledger.read().unwrap();
    assert_eq!(observed.state(), &state(2));
    assert!(ledger.writes_fenced());
    assert!(matches!(ledger.commit(&observed, &state(3)), Err(StoreError::WriteFenced)));
    let confirmed = ledger.reaffirm(&observed).unwrap();
    assert_eq!(confirmed.revision().get(), 3);
    assert_eq!(confirmed.state(), &state(2));
    assert!(!ledger.writes_fenced());
    drop(ledger);
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
